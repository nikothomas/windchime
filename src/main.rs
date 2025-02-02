mod demultiplex;
mod biom;

use clap::{Parser, Subcommand};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::path::{Path, PathBuf};
use std::env;
use std::sync::{Arc, Mutex};

// Add reqwest for downloading and flate2’s GzDecoder for unzipping.
use reqwest;
use flate2::read::GzDecoder;

// Use indicatif for progress reporting.
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::atomic::{AtomicBool, Ordering};

// GLOBAL VERBOSE FLAG: true = print commands verbosely, false = use progress bars.
static VERBOSE_MODE: AtomicBool = AtomicBool::new(false);

// OUTPUT DIRECTORY for all generated files.
const OUTPUT_DIR: &str = "windchime_out";

/// Helper to generate an output file or folder path within OUTPUT_DIR.
fn out_path(relative: &str) -> String {
    format!("{}/{}", OUTPUT_DIR, relative)
}

//
// Helper function to wrap each pipeline step with a progress spinner (when not verbose).
//
fn run_step<F>(description: &str, f: F) -> Result<(), Box<dyn Error>>
where
    F: FnOnce() -> Result<(), Box<dyn Error>>,
{
    if VERBOSE_MODE.load(Ordering::Relaxed) {
        println!("==> {}", description);
        f()
    } else {
        // Create a spinner-style progress bar with a harmonized style.
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("[{elapsed_precise}] {spinner:.cyan} {msg}")
        );
        pb.enable_steady_tick(100);
        pb.set_message(description.to_owned());
        let result = f();
        match &result {
            Ok(_) => pb.finish_with_message(format!("{} ✔", description)),
            Err(_) => pb.abandon_with_message(format!("{} ✘", description)),
        }
        result
    }
}

//
// 2) OS/arch detection and QIIME2 installation (unchanged except for verbosity)
//
fn conda_env_exists(env_name: &str) -> Result<bool, Box<dyn Error>> {
    let output = Command::new("conda")
        .arg("env")
        .arg("list")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("Failed to run 'conda env list': {}", e);
            return Err(e.into());
        }
    };

    if !output.status.success() {
        let msg = "Could not retrieve conda environment list.";
        eprintln!("{}", msg);
        return Err(msg.into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(stdout.contains(&format!(" {} ", env_name))
        || stdout.contains(&format!("/{env_name}\n"))
        || stdout.contains(&format!(" {}*", env_name)))
}

fn install_qiime2_amplicon_2024_10(env_name: &str) -> Result<(), Box<dyn Error>> {
    match conda_env_exists(env_name) {
        Ok(true) => {
            println!("Conda environment '{}' already exists. Skipping creation.", env_name);
            return Ok(());
        }
        Ok(false) => {} // continue with installation
        Err(e) => {
            eprintln!("Error checking conda environments: {}", e);
            return Err(e);
        }
    }

    let commands: Vec<String> = if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        vec![
            format!(
                "CONDA_SUBDIR=osx-64 conda env create -n {} --file https://data.qiime2.org/distro/amplicon/qiime2-amplicon-2024.10-py310-osx-conda.yml",
                env_name
            ),
            format!("conda activate {}", env_name),
            "conda config --env --set subdir osx-64".to_string(),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            format!(
                "conda env create -n {} --file https://data.qiime2.org/distro/amplicon/qiime2-amplicon-2024.10-py310-osx-conda.yml",
                env_name
            )
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            format!(
                "conda env create -n {} --file https://data.qiime2.org/distro/amplicon/qiime2-amplicon-2024.10-py310-linux-conda.yml",
                env_name
            )
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            format!(
                "conda env create -n {} --file https://data.qiime2.org/distro/amplicon/qiime2-amplicon-2024.10-py310-linux-conda.yml",
                env_name
            )
        ]
    } else {
        vec!["echo 'Unknown or unsupported platform'".to_string()]
    };
    println!("Installing QIIME 2 environment for your system...");
    for cmd in &commands {
        if let Err(e) = run_shell_command(cmd) {
            eprintln!("Error running command '{}': {}", cmd, e);
            return Err(e);
        }
    }
    println!("Installation complete. You can activate via: conda activate {}", env_name);
    Ok(())
}

//
// Modified command–execution functions check the global verbose flag.
//
fn run_shell_command(cmd: &str) -> Result<(), Box<dyn Error>> {
    if VERBOSE_MODE.load(Ordering::Relaxed) {
        println!("[CMD] {}", cmd);
    }
    let (stdout_setting, stderr_setting) = if VERBOSE_MODE.load(Ordering::Relaxed) {
        (Stdio::inherit(), Stdio::inherit())
    } else {
        (Stdio::null(), Stdio::null())
    };

    let status = Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(stdout_setting)
        .stderr(stderr_setting)
        .status();
    let status = match status {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to execute command '{}': {}", cmd, e);
            return Err(e.into());
        }
    };
    if !status.success() {
        let msg = format!("Command failed: {}", cmd);
        eprintln!("{}", msg);
        return Err(msg.into());
    }
    Ok(())
}

fn run_conda_qiime_command(env: &str, qiime_args: &str) -> Result<(), Box<dyn Error>> {
    if VERBOSE_MODE.load(Ordering::Relaxed) {
        println!("[QIIME CMD] qiime {}", qiime_args);
    }
    let mut args: Vec<&str> = vec!["run", "-n", env, "qiime"];
    args.extend(qiime_args.split_whitespace());
    let (stdout_setting, stderr_setting) = if VERBOSE_MODE.load(Ordering::Relaxed) {
        (Stdio::inherit(), Stdio::inherit())
    } else {
        (Stdio::null(), Stdio::null())
    };

    let status = Command::new("conda")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(stdout_setting)
        .stderr(stderr_setting)
        .status();
    let status = match status {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to run QIIME command 'qiime {}': {}", qiime_args, e);
            return Err(e.into());
        }
    };
    if !status.success() {
        let msg = format!("QIIME command failed: qiime {}", qiime_args);
        eprintln!("{}", msg);
        return Err(msg.into());
    }
    Ok(())
}

//
// 3) Download and Unzip Database Files
//
fn download_file(url: &str, output_path: &str, force: bool) -> Result<(), Box<dyn Error>> {
    if !force && Path::new(output_path).exists() {
        println!("File '{}' already exists, skipping download.", output_path);
        return Ok(());
    }
    println!("Downloading '{}' to '{}'...", url, output_path);
    let mut resp = reqwest::blocking::get(url)?;
    if !resp.status().is_success() {
        return Err(format!("Failed to download file: {}", url).into());
    }
    let mut out = File::create(output_path)?;
    io::copy(&mut resp, &mut out)?;
    Ok(())
}

fn unzip_file(input_path: &str, output_path: &str, force: bool) -> Result<(), Box<dyn Error>> {
    if !force && Path::new(output_path).exists() {
        println!("File '{}' already exists, skipping unzip.", output_path);
        return Ok(());
    }
    println!("Unzipping '{}' to '{}'...", input_path, output_path);
    let input_file = File::open(input_path)?;
    let mut gz = GzDecoder::new(input_file);
    let mut out = File::create(output_path)?;
    io::copy(&mut gz, &mut out)?;
    Ok(())
}

/// Downloads (and unzips when necessary) the required database files.
fn download_databases(force: bool) -> Result<(), Box<dyn Error>> {
    // Create directories for the databases inside OUTPUT_DIR.
    fs::create_dir_all(&out_path("db/pr2"))?;

    // --- pr2 database files (assumed to be provided as gzipped files) ---
    let pr2_fasta_url = "https://windchime.poleshift.cloud/pr2_version_5.0.0_SSU_mothur.fasta.gz";
    let pr2_tax_url   = "https://windchime.poleshift.cloud/pr2_version_5.0.0_SSU_mothur.tax";

    download_file(pr2_fasta_url, &out_path("db/pr2/pr2_with_taxonomy_simple.fasta.gz"), force)?;
    download_file(pr2_tax_url,   &out_path("db/pr2/pr2_taxonomy.tsv.gz"), force)?;

    // Unzip the pr2 files (if needed)
    unzip_file(&out_path("db/pr2/pr2_with_taxonomy_simple.fasta.gz"), &out_path("db/pr2/pr2_with_taxonomy_simple.fasta"), force)?;
    unzip_file(&out_path("db/pr2/pr2_taxonomy.tsv.gz"),              &out_path("db/pr2/pr2_taxonomy.tsv"), force)?;

    println!("Database download and extraction complete.");
    Ok(())
}

//
// 4) Full Pipeline Execution: Steps 2 – 7
//
// Note: The working directory is no longer changed. All outputs are placed in OUTPUT_DIR.
fn run_pipeline(
    env_name: &str,
    manifest: &str,
    metadata: &str,
    cores: usize,
    target: &str,
) -> Result<(), Box<dyn Error>> {
    // Ensure the main output directory exists.
    fs::create_dir_all(OUTPUT_DIR)?;

    // Choose adapter/primer sequences based on the target.
    let (adapter_f, adapter_r, primer_f, primer_r) = match target.to_lowercase().as_str() {
        "18s" => (
            "^TTGTACACACCGCCC...GTAGGTGAACCTGCRGAAGG",
            "^CCTTCYGCAGGTTCACCTAC...GGGCGGTGTGTACAA",
            "TTGTACACACCGCCC",
            "CCTTCYGCAGGTTCACCTAC"
        ),
        "16s" => (
            "^GTGYCAGCMGCCGCGGTAA...AAACTYAAAKRAATTGRCGG",
            "^CCGYCAATTYMTTTRAGTTT...TTACCGCGGCKGCTGRCAC",
            "GTGYCAGCMGCCGCGGTAA",
            "CCGYCAATTYMTTTRAGTTT"
        ),
        other => {
            eprintln!("Unsupported target: {}. Use '16s' or '18s'.", other);
            return Err("Unsupported target".into());
        }
    };

    // Step 2: Import Files using the generated manifest.
    run_step("Importing files using manifest", || {
        run_conda_qiime_command(env_name, &format!(
            "tools import --type SampleData[PairedEndSequencesWithQuality] \
         --input-path {} \
         --output-path {} \
         --input-format PairedEndFastqManifestPhred33V2",
            out_path(&manifest), // use out_path here!
            out_path("paired-end-demux.qza")
        ))
    })?;
    run_step("Validating imported file", || {
        run_conda_qiime_command(env_name, &format!(
            "tools validate {}",
            out_path("paired-end-demux.qza")
        ))
    })?;
    run_step("Summarizing demultiplexed data", || {
        run_conda_qiime_command(env_name, &format!(
            "demux summarize --i-data {} --o-visualization {}",
            out_path("paired-end-demux.qza"),
            out_path("paired-end-demux.qzv")
        ))
    })?;

    // Step 3: Trim Reads with Cutadapt.
    let cutadapt_command = format!(
        "cutadapt trim-paired --i-demultiplexed-sequences {}  \
         --p-cores {} --p-adapter-f {} --p-adapter-r {} \
         --p-error-rate 0.1 --p-overlap 3 --verbose \
         --o-trimmed-sequences {}",
        out_path("paired-end-demux.qza"),
        cores, adapter_f, adapter_r,
        out_path("paired-end-demux-trimmed.qza")
    );
    run_step("Trimming reads with Cutadapt", || {
        run_conda_qiime_command(env_name, &cutadapt_command)
    })?;
    run_step("Summarizing trimmed data", || {
        run_conda_qiime_command(env_name, &format!(
            "demux summarize --i-data {} --p-n 100000 --o-visualization {}",
            out_path("paired-end-demux-trimmed.qza"),
            out_path("paired-end-demux-trimmed.qzv")
        ))
    })?;

    // Step 4: Denoise with DADA2.
    run_step("Creating directory for DADA2 output", || {
        fs::create_dir_all(&out_path("asvs")).map_err(|e| e.into())
    })?;
    run_step("Running DADA2 denoise-paired", || {
        run_conda_qiime_command(env_name, &format!(
            "dada2 denoise-paired \
             --i-demultiplexed-seqs {} \
             --p-n-threads 0 --p-trunc-q 2 --p-trunc-len-f 219 --p-trunc-len-r 194 \
             --p-max-ee-f 2 --p-max-ee-r 4 --p-n-reads-learn 1000000 \
             --p-chimera-method pooled \
             --o-table {} \
             --o-representative-sequences {} \
             --o-denoising-stats {}",
            out_path("paired-end-demux-trimmed.qza"),
            out_path("asvs/table-dada2.qza"),
            out_path("asvs/rep-seqs-dada2.qza"),
            out_path("asvs/stats-dada2.qza")
        ))
    })?;
    run_step("Tabulating DADA2 denoising stats", || {
        run_conda_qiime_command(env_name, &format!(
            "metadata tabulate --m-input-file {} --o-visualization {}",
            out_path("asvs/stats-dada2.qza"),
            out_path("asvs/stats-dada2.qzv")
        ))
    })?;

    // Step 5: Export and Summarize Denoised Data.
    run_step("Exporting ASV table", || {
        run_conda_qiime_command(env_name, &format!(
            "tools export --input-path {} --output-path {}",
            out_path("asvs/table-dada2.qza"),
            out_path("asv_table")
        ))
    })?;
    run_step("Converting BIOM to TSV", || {
        biom::convert_biom_to_tsv(
            &format!("{}/feature-table.biom", out_path("asv_table")),
            &format!("{}/asv-table.tsv", out_path("asv_table"))
        )
    })?;
    run_step("Exporting representative sequences", || {
        run_conda_qiime_command(env_name, &format!(
            "tools export --input-path {} --output-path {}",
            out_path("asvs/rep-seqs-dada2.qza"),
            out_path("asvs")
        ))
    })?;
    run_step("Tabulating representative sequences", || {
        run_conda_qiime_command(env_name, &format!(
            "feature-table tabulate-seqs --i-data {} --o-visualization {}",
            out_path("asvs/rep-seqs-dada2.qza"),
            out_path("asvs/rep-seqs-dada2.qzv")
        ))
    })?;
    run_step("Summarizing feature table", || {
        run_conda_qiime_command(env_name, &format!(
            "feature-table summarize --i-table {} --o-visualization {} --m-sample-metadata-file {}",
            out_path("asvs/table-dada2.qza"),
            out_path("asvs/table-dada2.qzv"),
            metadata
        ))
    })?;

    // Step 6: Taxonomic Annotation using pr2.
    run_step("Creating directory for pr2 database output", || {
        fs::create_dir_all(&out_path("db/pr2")).map_err(|e| e.into())
    })?;
    run_step("Importing pr2 sequences", || {
        run_conda_qiime_command(env_name, &format!(
            "tools import --type FeatureData[Sequence] \
             --input-path {} \
             --output-path {}",
            out_path("db/pr2/pr2_with_taxonomy_simple.fasta"),
            out_path("db/pr2/pr2.qza")
        ))
    })?;
    run_step("Importing pr2 taxonomy", || {
        run_conda_qiime_command(env_name, &format!(
            "tools import --type FeatureData[Taxonomy] \
             --input-format HeaderlessTSVTaxonomyFormat \
             --input-path {} \
             --output-path {}",
            out_path("db/pr2/pr2_taxonomy.tsv"),
            out_path("db/pr2/pr2_tax.qza")
        ))
    })?;
    run_step("Extracting pr2 reads", || {
        let extract_reads_command = format!(
            "feature-classifier extract-reads \
             --i-sequences {} \
             --p-f-primer {} \
             --p-r-primer {} \
             --o-reads {}",
            out_path("db/pr2/pr2.qza"), primer_f, primer_r, out_path("db/pr2/pr2_extracts.qza")
        );
        run_conda_qiime_command(env_name, &extract_reads_command)
    })?;
    run_step("Fitting pr2 classifier", || {
        run_conda_qiime_command(env_name, &format!(
            "feature-classifier fit-classifier-naive-bayes \
             --i-reference-reads {} \
             --i-reference-taxonomy {} \
             --o-classifier {}",
            out_path("db/pr2/pr2_extracts.qza"),
            out_path("db/pr2/pr2_tax.qza"),
            out_path("db/pr2/pr2_classifier.qza")
        ))
    })?;
    run_step("Classifying reads with pr2 classifier", || {
        run_conda_qiime_command(env_name, &format!(
            "feature-classifier classify-sklearn \
             --p-n-jobs -1 \
             --i-classifier {} \
             --i-reads {} \
             --o-classification {}",
            out_path("db/pr2/pr2_classifier.qza"),
            out_path("asvs/rep-seqs-dada2.qza"),
            out_path("pr2_tax_sklearn.qza")
        ))
    })?;
    run_step("Exporting pr2 taxonomy", || {
        run_conda_qiime_command(env_name, &format!(
            "tools export --input-path {} --output-path {}",
            out_path("pr2_tax_sklearn.qza"),
            out_path("asv_tax_dir")
        ))
    })?;
    run_step("Renaming pr2 taxonomy file", || {
        run_shell_command(&format!(
            "mv {}/taxonomy.tsv {}/pr2_taxonomy.tsv",
            out_path("asv_tax_dir"),
            out_path("asv_tax_dir")
        ))
    })?;

    // Step 7: Merge ASV Table with Taxonomy.
    run_step("Merging ASV and taxonomy tables", || {
        merge_asv_taxonomy()
    })?;

    println!("\nPipeline completed successfully!");
    Ok(())
}

//
// 5) Merge ASV and Taxonomy Tables in Rust (instead of using R)
//
fn merge_asv_taxonomy() -> Result<(), Box<dyn Error>> {
    use std::collections::HashMap;
    use csv::{ReaderBuilder, WriterBuilder};

    // Read the ASV table TSV.
    let asv_reader_result = ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .from_path(out_path("asv_table/asv-table.tsv"));
    let mut asv_reader = match asv_reader_result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error reading ASV table: {}", e);
            return Err(e.into());
        }
    };
    let asv_headers = match asv_reader.headers() {
        Ok(h) => h.clone(),
        Err(e) => {
            eprintln!("Error reading ASV table headers: {}", e);
            return Err(e.into());
        }
    };
    let mut asv_map: HashMap<String, Vec<String>> = HashMap::new();
    for result in asv_reader.records() {
        match result {
            Ok(record) => {
                let feature_id = record.get(0).unwrap_or("").to_string();
                asv_map.insert(feature_id, record.iter().map(|s| s.to_string()).collect());
            }
            Err(e) => {
                eprintln!("Error reading record from ASV table: {}", e);
                return Err(e.into());
            }
        }
    }

    // Read pr2 taxonomy table.
    let pr2_reader_result = ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .from_path(out_path("asv_tax_dir/pr2_taxonomy.tsv"));
    let mut pr2_reader = match pr2_reader_result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error reading pr2 taxonomy table: {}", e);
            return Err(e.into());
        }
    };
    let pr2_headers = match pr2_reader.headers() {
        Ok(h) => h.clone(),
        Err(e) => {
            eprintln!("Error reading pr2 table headers: {}", e);
            return Err(e.into());
        }
    };
    let mut pr2_map: HashMap<String, Vec<String>> = HashMap::new();
    for result in pr2_reader.records() {
        match result {
            Ok(record) => {
                let feature_id = record.get(0).unwrap_or("").to_string();
                pr2_map.insert(feature_id, record.iter().map(|s| s.to_string()).collect());
            }
            Err(e) => {
                eprintln!("Error reading record from pr2 table: {}", e);
                return Err(e.into());
            }
        }
    }

    // Prepare output writer.
    let output_result = WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(out_path("asv_count_tax.tsv"));
    let mut wtr = match output_result {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Error creating output file asv_count_tax.tsv: {}", e);
            return Err(e.into());
        }
    };

    // Build merged header.
    let mut merged_header = Vec::new();
    for (i, col) in asv_headers.iter().enumerate() {
        if i == 0 {
            merged_header.push("Feature.ID".to_string());
        } else {
            merged_header.push(col.to_string());
        }
    }
    for (i, col) in pr2_headers.iter().enumerate() {
        if i == 0 { continue; }
        merged_header.push(format!("pr2_{}", col));
    }
    if let Err(e) = wtr.write_record(&merged_header) {
        eprintln!("Error writing header to output file: {}", e);
        return Err(e.into());
    }

    // For each ASV record, add taxonomy from pr2.
    for (feature_id, asv_record) in asv_map.iter() {
        let mut merged_record = asv_record.clone();

        if let Some(pr2_record) = pr2_map.get(feature_id) {
            merged_record.extend(pr2_record.iter().skip(1).cloned());
        } else {
            for _ in 1..pr2_headers.len() {
                merged_record.push(String::new());
            }
        }
        if let Err(e) = wtr.write_record(&merged_record) {
            eprintln!("Error writing record for feature {}: {}", feature_id, e);
            return Err(e.into());
        }
    }

    if let Err(e) = wtr.flush() {
        eprintln!("Error flushing output file: {}", e);
        return Err(e.into());
    }
    println!("Merged ASV count and taxonomy table written to {}", out_path("asv_count_tax.tsv"));
    Ok(())
}

//
// 6) CLI Definitions
//
#[derive(Parser, Debug)]
#[command(name = "windchime", about = "A Rust CLI for QIIME2 16S/18S pipeline")]
struct Cli {
    /// Enable verbose output: print the full QIIME command for each step.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Install (or skip if existing) the specified QIIME2 environment.
    InstallEnv {
        #[arg(short, long, default_value = "qiime2-amplicon-2024.10")]
        env_name: String,
    },
    /// Run demultiplexing using a barcodes file.
    Demux {
        /// Path to the barcodes file for demultiplexing.
        barcodes_file: PathBuf,
    },
    /// Execute only Steps 2–7 of the pipeline.
    Pipeline {
        #[arg(short, long, default_value = "qiime2-amplicon-2024.10")]
        env_name: String,
        /// QIIME2 manifest file.
        #[arg(short, long, default_value = "manifest.tsv")]
        manifest: String,
        #[arg(short, long, default_value = "metadata.tsv")]
        metadata: String,
        #[arg(long, default_value_t = 1)]
        cores: usize,
        /// Target region: choose "16s" or "18s" (default: 18s)
        #[arg(short, long, default_value = "18s", value_parser = ["16s", "18s"])]
        target: String,
    },
    /// Single command: install environment if needed, demultiplex using a barcodes file,
    /// generate a QIIME2 manifest file, and run the pipeline.
    RunAll {
        #[arg(short, long, default_value = "qiime2-amplicon-2024.10")]
        env_name: String,
        /// Path to the barcodes file for demultiplexing.
        #[arg(long, default_value = "barcodes.tsv")]
        barcodes_file: String,
        /// (Optional) QIIME2 manifest file (if not provided, it will be generated).
        #[arg(short, long, default_value = "manifest.tsv")]
        manifest: String,
        #[arg(short, long, default_value = "metadata.tsv")]
        metadata: String,
        #[arg(long, default_value_t = 1)]
        cores: usize,
        /// Target region: choose "16s" or "18s" (default: 18s)
        #[arg(short, long, default_value = "18s", value_parser = ["16s", "18s"])]
        target: String,
    },
    /// Download the database files (and unzip them if needed).
    DownloadDBs {
        /// Force re-download and unzip even if the files already exist.
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
}

//
// 7) Main Program Entry Point
//
fn main() {
    let cli = Cli::parse();

    // Set the global verbose flag.
    VERBOSE_MODE.store(cli.verbose, Ordering::Relaxed);

    // Ensure that the output directory exists.
    if let Err(e) = fs::create_dir_all(OUTPUT_DIR) {
        eprintln!("Error creating output directory {}: {}", OUTPUT_DIR, e);
        std::process::exit(1);
    }

    let result = match cli.command {
        Commands::InstallEnv { env_name } => {
            install_qiime2_amplicon_2024_10(&env_name)
        }
        Commands::Demux { barcodes_file } => {
            let barcode_str = match barcodes_file.to_str() {
                Some(s) => s,
                None => {
                    eprintln!("Error: Invalid barcodes file path");
                    return;
                }
            };
            demultiplex::run_demultiplex_combined(barcode_str)
                .map_err(|e| e.into())
        }
        Commands::Pipeline {
            env_name,
            manifest,
            metadata,
            cores,
            target,
        } => {
            println!("Running QIIME2 pipeline with environment: {}", env_name);
            run_pipeline(&env_name, &manifest, &metadata, cores, &target)
        }
        Commands::RunAll {
            env_name,
            barcodes_file,
            manifest,
            metadata,
            cores,
            target,
        } => {
            println!("==> Checking conda environment '{}'", env_name);
            if let Err(e) = install_qiime2_amplicon_2024_10(&env_name) {
                eprintln!("Error installing environment: {}", e);
                return;
            }

            println!("==> Running demultiplexing step...");
            if let Err(e) = demultiplex::run_demultiplex_combined(&barcodes_file) {
                eprintln!("Error in demultiplexing: {}", e);
                return;
            }

            println!("==> Generating QIIME2 manifest file...");
            if let Err(e) = demultiplex::generate_qiime_manifest(&barcodes_file, &manifest) {
                eprintln!("Error generating QIIME manifest: {}", e);
                return;
            }

            println!("==> Running QIIME2 pipeline using manifest file: {}", manifest);
            run_pipeline(&env_name, &manifest, &metadata, cores, &target)
        }
        Commands::DownloadDBs { force } => {
            if let Err(e) = download_databases(force) {
                eprintln!("Error downloading databases: {}", e);
                std::process::exit(1);
            }
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Application error: {}", e);
        std::process::exit(1);
    }
}
