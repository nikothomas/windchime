mod demultiplex;

use clap::{Parser, Subcommand};
use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest;
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{env, time::Duration};

// GLOBAL VERBOSE FLAG: true = print commands verbosely, false = use progress bars.
static VERBOSE_MODE: AtomicBool = AtomicBool::new(false);

// OUTPUT DIRECTORY for all generated files.
const OUTPUT_DIR: &str = "windchime_out";

/// Helper to generate an output file or folder path within OUTPUT_DIR.
fn out_path(relative: &str) -> String {
    format!("{}/{}", OUTPUT_DIR, relative)
}

/// Wraps an operation `f` in a spinner-based progress bar if not in verbose mode.
/// Otherwise, simply prints the description and runs `f`.
fn run_step<F>(description: &str, f: F) -> Result<(), Box<dyn Error>>
where
    F: FnOnce() -> Result<(), Box<dyn Error>>,
{
    // If verbose, just print the step description and run it directly.
    if VERBOSE_MODE.load(Ordering::Relaxed) {
        println!("==> {}", description);
        return f();
    }

    // Otherwise, create a spinner progress bar.
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("[{elapsed_precise}] {spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&[
                "⡿","⠿","⢟","⠟","⡛","⠛","⠫","⢋","⠋","⠍","⡉","⠉","⠑","⠡","⢁",
                "⠁","⠂","⠄","⡀","⡈","⡐","⡠","⣀","⣁","⣂","⣄","⣌","⣔","⣤",
                "⣥","⣦","⣮","⣶","⣷","⣿"
            ]),
    );
    pb.enable_steady_tick(Duration::new(0, 100_000_000));
    pb.set_message(description.to_owned());

    let result = f();
    match &result {
        Ok(_) => pb.finish_with_message(format!("{} ✔", description)),
        Err(_) => pb.abandon_with_message(format!("{} ✘", description)),
    }
    result
}

/// Checks if a specified conda environment already exists.
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Heuristic check if the environment name appears in the conda env list output.
    Ok(
        stdout.contains(&format!(" {} ", env_name))
            || stdout.contains(&format!("/{env_name}\n"))
            || stdout.contains(&format!(" {}*", env_name)),
    )
}

/// Installs the specified QIIME2 environment if it doesn’t already exist.
fn install_qiime2_amplicon_2024_10(env_name: &str) -> Result<(), Box<dyn Error>> {
    match conda_env_exists(env_name) {
        Ok(true) => {
            println!("Conda environment '{}' already exists. Skipping creation.", env_name);
            return Ok(());
        }
        Ok(false) => {} // proceed
        Err(e) => {
            eprintln!("Error checking conda environments: {}", e);
            return Err(e);
        }
    }

    // Use different .yml files depending on OS and architecture.
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
        vec![format!(
            "conda env create -n {} --file https://data.qiime2.org/distro/amplicon/qiime2-amplicon-2024.10-py310-osx-conda.yml",
            env_name
        )]
    } else if cfg!(target_os = "linux") {
        vec![format!(
            "conda env create -n {} --file https://data.qiime2.org/distro/amplicon/qiime2-amplicon-2024.10-py310-linux-conda.yml",
            env_name
        )]
    } else if cfg!(target_os = "windows") {
        // Windows uses the Linux .yml in WSL or a Miniconda-like setup.
        vec![format!(
            "conda env create -n {} --file https://data.qiime2.org/distro/amplicon/qiime2-amplicon-2024.10-py310-linux-conda.yml",
            env_name
        )]
    } else {
        vec!["echo 'Unknown or unsupported platform'".to_string()]
    };

    println!("Installing QIIME2 environment for your system...");
    for cmd in &commands {
        if let Err(e) = run_shell_command(cmd) {
            eprintln!("Error running command '{}': {}", cmd, e);
            return Err(e);
        }
    }
    println!(
        "Installation complete. You can activate via: conda activate {}",
        env_name
    );
    Ok(())
}

/// Executes a shell command (via `bash -c`) in either quiet or verbose mode.
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

/// Runs a QIIME command in a specified conda environment via `conda run`.
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

/// Converts a BIOM file into TSV format by calling `biom convert` via conda.
fn convert_biom_to_tsv_conda(
    env_name: &str,
    biom_in: &str,
    tsv_out: &str,
) -> Result<(), Box<dyn Error>> {
    let cmd = format!(
        "conda run -n {} biom convert -i {} -o {} --to-tsv",
        env_name, biom_in, tsv_out
    );
    run_shell_command(&cmd)
}

/// Downloads a file from a URL to an output path. If `force` is false,
/// skips download if the file already exists.
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

/// Unzips a `.gz` file to `output_path`. If `force` is false,
/// skips unzip if `output_path` already exists.
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

/// Downloads (and unzips) the required database files into `OUTPUT_DIR/db/pr2`.
fn download_databases(force: bool) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(out_path("db/pr2"))?;

    // --- pr2 database files ---
    let pr2_fasta_url = "https://windchime.poleshift.cloud/pr2_version_5.0.0_SSU_mothur.fasta.gz";
    let pr2_tax_url   = "https://windchime.poleshift.cloud/pr2_version_5.0.0_SSU_mothur.tax.gz";

    download_file(pr2_fasta_url, &out_path("db/pr2/pr2_with_taxonomy_simple.fasta.gz"), force)?;
    download_file(pr2_tax_url,   &out_path("db/pr2/pr2_taxonomy.tsv.gz"), force)?;

    unzip_file(
        &out_path("db/pr2/pr2_with_taxonomy_simple.fasta.gz"),
        &out_path("db/pr2/pr2_with_taxonomy_simple.fasta"),
        force,
    )?;
    unzip_file(
        &out_path("db/pr2/pr2_taxonomy.tsv.gz"),
        &out_path("db/pr2/pr2_taxonomy.tsv"),
        force,
    )?;

    println!("Database download and extraction complete.");
    Ok(())
}

/// Primary pipeline function: runs Steps 2–7 of the QIIME2 workflow, optionally skipping existing outputs.
fn run_pipeline(
    env_name: &str,
    manifest: &str,
    metadata: &str,
    cores: usize,
    target: &str,
    skip_existing: bool,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(OUTPUT_DIR)?;

    // Set up adapter/primer sequences based on the target.
    let (adapter_f, adapter_r, primer_f, primer_r) = match target.to_lowercase().as_str() {
        "18s" => (
            "^TTGTACACACCGCCC...GTAGGTGAACCTGCRGAAGG",
            "^CCTTCYGCAGGTTCACCTAC...GGGCGGTGTGTACAA",
            "TTGTACACACCGCCC",
            "CCTTCYGCAGGTTCACCTAC",
        ),
        "16s" => (
            "^GTGYCAGCMGCCGCGGTAA...AAACTYAAAKRAATTGRCGG",
            "^CCGYCAATTYMTTTRAGTTT...TTACCGCGGCKGCTGRCAC",
            "GTGYCAGCMGCCGCGGTAA",
            "CCGYCAATTYMTTTRAGTTT",
        ),
        other => {
            eprintln!("Unsupported target: {}. Use '16s' or '18s'.", other);
            return Err("Unsupported target".into());
        }
    };

    // Step 2: Import Files using the manifest
    let pe_demux_qza = out_path("paired-end-demux.qza");
    if skip_existing && Path::new(&pe_demux_qza).exists() {
        println!("Skipping 'Importing files' because '{}' already exists.", pe_demux_qza);
    } else {
        run_step("Importing files using manifest", || {
            run_conda_qiime_command(env_name, &format!(
                "tools import --type SampleData[PairedEndSequencesWithQuality] \
                 --input-path {} \
                 --output-path {} \
                 --input-format PairedEndFastqManifestPhred33V2",
                out_path(manifest),
                pe_demux_qza
            ))
        })?;
    }

    // Summarize the demultiplexed data
    let pe_demux_qzv = out_path("paired-end-demux.qzv");
    if skip_existing && Path::new(&pe_demux_qzv).exists() {
        println!(
            "Skipping 'Summarizing demultiplexed data' because '{}' already exists.",
            pe_demux_qzv
        );
    } else {
        run_step("Validating imported file", || {
            run_conda_qiime_command(env_name, &format!("tools validate {}", pe_demux_qza))
        })?;
        run_step("Summarizing demultiplexed data", || {
            run_conda_qiime_command(env_name, &format!(
                "demux summarize --i-data {} --o-visualization {}",
                pe_demux_qza, pe_demux_qzv
            ))
        })?;
    }

    // Step 3: Trim Reads with Cutadapt
    let pe_trimmed_qza = out_path("paired-end-demux-trimmed.qza");
    let pe_trimmed_qzv = out_path("paired-end-demux-trimmed.qzv");
    if skip_existing && Path::new(&pe_trimmed_qza).exists() && Path::new(&pe_trimmed_qzv).exists() {
        println!(
            "Skipping 'Trimming reads with Cutadapt' because '{}' already exists.",
            pe_trimmed_qza
        );
    } else {
        run_step("Trimming reads with Cutadapt", || {
            let cutadapt_command = format!(
                "cutadapt trim-paired --i-demultiplexed-sequences {}  \
                 --p-cores {} --p-adapter-f {} --p-adapter-r {} \
                 --p-error-rate 0.1 --p-overlap 3 --verbose \
                 --o-trimmed-sequences {}",
                pe_demux_qza, cores, adapter_f, adapter_r, pe_trimmed_qza
            );
            run_conda_qiime_command(env_name, &cutadapt_command)
        })?;
        run_step("Summarizing trimmed data", || {
            run_conda_qiime_command(env_name, &format!(
                "demux summarize --i-data {} --p-n 100000 --o-visualization {}",
                pe_trimmed_qza, pe_trimmed_qzv
            ))
        })?;
    }

    // Step 4: Denoise with DADA2
    let asvs_dir = out_path("asvs");
    let table_dada2_qza = out_path("asvs/table-dada2.qza");
    let rep_seqs_dada2_qza = out_path("asvs/rep-seqs-dada2.qza");
    let stats_dada2_qza = out_path("asvs/stats-dada2.qza");
    if skip_existing
        && Path::new(&table_dada2_qza).exists()
        && Path::new(&rep_seqs_dada2_qza).exists()
        && Path::new(&stats_dada2_qza).exists()
    {
        println!("Skipping 'DADA2 denoise-paired' because output files already exist.");
    } else {
        run_step("Creating directory for DADA2 output", || {
            fs::create_dir_all(&asvs_dir).map_err(|e| e.into())
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
                pe_trimmed_qza, table_dada2_qza, rep_seqs_dada2_qza, stats_dada2_qza
            ))
        })?;
        run_step("Tabulating DADA2 denoising stats", || {
            run_conda_qiime_command(env_name, &format!(
                "metadata tabulate --m-input-file {} --o-visualization {}",
                stats_dada2_qza,
                out_path("asvs/stats-dada2.qzv")
            ))
        })?;
    }

    // Step 5: Export and Summarize Denoised Data
    let asv_table_dir = out_path("asv_table");
    if skip_existing && Path::new(&format!("{}/feature-table.biom", asv_table_dir)).exists() {
        println!("Skipping 'Exporting ASV table' because feature-table.biom already exists.");
    } else {
        run_step("Exporting ASV table", || {
            run_conda_qiime_command(env_name, &format!(
                "tools export --input-path {} --output-path {}",
                table_dada2_qza, asv_table_dir
            ))
        })?;
    }

    if skip_existing && Path::new(&format!("{}/asv-table.tsv", asv_table_dir)).exists() {
        println!("Skipping 'Converting BIOM to TSV' because asv-table.tsv already exists.");
    } else {
        run_step("Converting BIOM to TSV", || {
            convert_biom_to_tsv_conda(
                env_name,
                &format!("{}/feature-table.biom", asv_table_dir),
                &format!("{}/asv-table.tsv", asv_table_dir),
            )
        })?;
    }

    let rep_seqs_export_dir = out_path("asvs"); // same dir for rep seqs export
    if skip_existing && Path::new(&format!("{}/dna-sequences.fasta", rep_seqs_export_dir)).exists() {
        println!(
            "Skipping 'Exporting representative sequences' because dna-sequences.fasta already exists."
        );
    } else {
        run_step("Exporting representative sequences", || {
            run_conda_qiime_command(env_name, &format!(
                "tools export --input-path {} --output-path {}",
                rep_seqs_dada2_qza, rep_seqs_export_dir
            ))
        })?;
    }

    let rep_seqs_dada2_qzv = out_path("asvs/rep-seqs-dada2.qzv");
    if skip_existing && Path::new(&rep_seqs_dada2_qzv).exists() {
        println!(
            "Skipping 'Tabulating representative sequences' because '{}' already exists.",
            rep_seqs_dada2_qzv
        );
    } else {
        run_step("Tabulating representative sequences", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-table tabulate-seqs --i-data {} --o-visualization {}",
                rep_seqs_dada2_qza, rep_seqs_dada2_qzv
            ))
        })?;
    }

    let table_dada2_qzv = out_path("asvs/table-dada2.qzv");
    if skip_existing && Path::new(&table_dada2_qzv).exists() {
        println!(
            "Skipping 'Summarizing feature table' because '{}' already exists.",
            table_dada2_qzv
        );
    } else {
        run_step("Summarizing feature table", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-table summarize --i-table {} --o-visualization {}",
                table_dada2_qza, table_dada2_qzv
            ))
        })?;
    }

    // Step 6: Taxonomic Annotation using pr2
    let pr2_dir = out_path("db/pr2");
    if skip_existing && Path::new(&out_path("db/pr2/pr2.qza")).exists() {
        println!("Skipping 'Importing pr2 sequences' because 'pr2.qza' already exists.");
    } else {
        run_step("Creating directory for pr2 database output", || {
            fs::create_dir_all(&pr2_dir).map_err(|e| e.into())
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
    }

    if skip_existing && Path::new(&out_path("db/pr2/pr2_tax.qza")).exists() {
        println!("Skipping 'Importing pr2 taxonomy' because 'pr2_tax.qza' already exists.");
    } else {
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
    }

    let pr2_extracts_qza = out_path("db/pr2/pr2_extracts.qza");
    if skip_existing && Path::new(&pr2_extracts_qza).exists() {
        println!("Skipping 'Extracting pr2 reads' because '{}' already exists.", pr2_extracts_qza);
    } else {
        run_step("Extracting pr2 reads", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-classifier extract-reads \
                 --i-sequences {} \
                 --p-f-primer {} \
                 --p-r-primer {} \
                 --o-reads {}",
                out_path("db/pr2/pr2.qza"), primer_f, primer_r, pr2_extracts_qza
            ))
        })?;
    }

    let pr2_classifier_qza = out_path("db/pr2/pr2_classifier.qza");
    if skip_existing && Path::new(&pr2_classifier_qza).exists() {
        println!("Skipping 'Fitting pr2 classifier' because '{}' already exists.", pr2_classifier_qza);
    } else {
        run_step("Fitting pr2 classifier", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-classifier fit-classifier-naive-bayes \
                 --i-reference-reads {} \
                 --i-reference-taxonomy {} \
                 --o-classifier {} \
                 --p-classify--chunk-size 100000",
                pr2_extracts_qza,
                out_path("db/pr2/pr2_tax.qza"),
                pr2_classifier_qza
            ))
        })?;
    }

    let pr2_tax_sklearn_qza = out_path("pr2_tax_sklearn.qza");
    if skip_existing && Path::new(&pr2_tax_sklearn_qza).exists() {
        println!(
            "Skipping 'Classifying reads with pr2 classifier' because '{}' already exists.",
            pr2_tax_sklearn_qza
        );
    } else {
        run_step("Classifying reads with pr2 classifier", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-classifier classify-sklearn \
                 --p-n-jobs 0 \
                 --i-classifier {} \
                 --i-reads {} \
                 --o-classification {}",
                pr2_classifier_qza, rep_seqs_dada2_qza, pr2_tax_sklearn_qza
            ))
        })?;
    }

    let asv_tax_dir = out_path("asv_tax_dir");
    if skip_existing && Path::new(&format!("{}/taxonomy.tsv", asv_tax_dir)).exists() {
        println!("Skipping 'Exporting pr2 taxonomy' because 'taxonomy.tsv' already exists.");
    } else {
        run_step("Exporting pr2 taxonomy", || {
            run_conda_qiime_command(env_name, &format!(
                "tools export --input-path {} --output-path {}",
                pr2_tax_sklearn_qza, asv_tax_dir
            ))
        })?;
    }

    let pr2_taxonomy_tsv = format!("{}/pr2_taxonomy.tsv", asv_tax_dir);
    if skip_existing && Path::new(&pr2_taxonomy_tsv).exists() {
        println!("Skipping 'Renaming pr2 taxonomy file' because '{}' already exists.", pr2_taxonomy_tsv);
    } else {
        run_step("Renaming pr2 taxonomy file", || {
            run_shell_command(&format!("mv {}/taxonomy.tsv {}", asv_tax_dir, pr2_taxonomy_tsv))
        })?;
    }

    // Step 7: Merge ASV Table with Taxonomy
    let merged_output = out_path("asv_count_tax.tsv");
    if skip_existing && Path::new(&merged_output).exists() {
        println!(
            "Skipping 'Merging ASV and taxonomy tables' because '{}' already exists.",
            merged_output
        );
    } else {
        run_step("Merging ASV and taxonomy tables", merge_asv_taxonomy)?;
    }

    println!("\nPipeline completed successfully!");
    Ok(())
}

/// Merges the ASV count table with the assigned taxonomy, producing `asv_count_tax.tsv`.
fn merge_asv_taxonomy() -> Result<(), Box<dyn Error>> {
    use csv::{ReaderBuilder, WriterBuilder};

    // Read the ASV table (TSV).
    let mut asv_reader = ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .comment(Some(b'#'))
        .from_path(out_path("asv_table/asv-table.tsv"))?;

    let asv_headers = asv_reader.headers()?.clone();
    let mut asv_map: HashMap<String, Vec<String>> = HashMap::new();

    for record in asv_reader.records() {
        let rec = record?;
        let feature_id = rec.get(0).unwrap_or("").to_string();
        asv_map.insert(feature_id, rec.iter().map(|s| s.to_string()).collect());
    }

    // Read the pr2 taxonomy table (TSV).
    let mut pr2_reader = ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .from_path(out_path("asv_tax_dir/pr2_taxonomy.tsv"))?;

    let pr2_headers = pr2_reader.headers()?.clone();
    let mut pr2_map: HashMap<String, Vec<String>> = HashMap::new();

    for record in pr2_reader.records() {
        let rec = record?;
        let feature_id = rec.get(0).unwrap_or("").to_string();
        pr2_map.insert(feature_id, rec.iter().map(|s| s.to_string()).collect());
    }

    // Prepare an output writer for the merged TSV.
    let mut wtr = WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(out_path("asv_count_tax.tsv"))?;

    // Build the merged header.
    let mut merged_header = Vec::new();
    for (i, col) in asv_headers.iter().enumerate() {
        if i == 0 {
            merged_header.push("Feature.ID".to_string());
        } else {
            merged_header.push(col.to_string());
        }
    }
    for (i, col) in pr2_headers.iter().enumerate() {
        if i == 0 {
            // skip feature ID column from the pr2 file
            continue;
        }
        merged_header.push(format!("pr2_{}", col));
    }
    wtr.write_record(&merged_header)?;

    // For each ASV record, append taxonomy columns if present.
    for (feature_id, asv_record) in asv_map.iter() {
        let mut merged_record = asv_record.clone();
        if let Some(pr2_record) = pr2_map.get(feature_id) {
            // Skip the first column (feature ID), append the rest.
            merged_record.extend(pr2_record.iter().skip(1).cloned());
        } else {
            // If there's no matching taxonomy, fill with empty strings.
            for _ in 1..pr2_headers.len() {
                merged_record.push(String::new());
            }
        }
        wtr.write_record(&merged_record)?;
    }
    wtr.flush()?;

    println!(
        "Merged ASV count and taxonomy table written to {}",
        out_path("asv_count_tax.tsv")
    );
    Ok(())
}

#[derive(Parser, Debug)]
#[command(name = "windchime", about = "A Rust CLI for QIIME2 16S/18S pipeline")]
struct Cli {
    /// Enable verbose output: print the full QIIME command for each step
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
    /// Execute only Steps 2–7 of the pipeline, optionally skipping existing outputs.
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
        /// If true, skip any steps whose expected QIIME artifacts already exist
        #[arg(long, default_value_t = false)]
        skip_existing: bool,
    },
    /// Single command: install environment if needed, demultiplex using a barcodes file,
    /// generate a QIIME2 manifest file, download DBs, and run the pipeline.
    RunAll {
        #[arg(short, long, default_value = "qiime2-amplicon-2024.10")]
        env_name: String,
        /// Path to the barcodes file for demultiplexing.
        #[arg(long, default_value = "barcodes.tsv")]
        barcodes_file: String,
        /// QIIME2 manifest file (if not provided, it will be generated).
        #[arg(short, long, default_value = "manifest.tsv")]
        manifest: String,
        #[arg(short, long, default_value = "metadata.tsv")]
        metadata: String,
        #[arg(long, default_value_t = 1)]
        cores: usize,
        /// Target region: choose "16s" or "18s" (default: 18s)
        #[arg(short, long, default_value = "18s", value_parser = ["16s", "18s"])]
        target: String,
        /// If true, skip any steps whose expected QIIME artifacts already exist
        #[arg(long, default_value_t = false)]
        skip_existing: bool,
    },
    /// Download the database files (and unzip them if needed).
    DownloadDBs {
        /// Force re-download and unzip even if the files already exist.
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
}

/// Main entry point for the Windchime CLI.
fn main() {
    let cli = Cli::parse();

    // Set the global verbose flag.
    VERBOSE_MODE.store(cli.verbose, Ordering::Relaxed);

    // Ensure the output directory exists (create if missing).
    if let Err(e) = fs::create_dir_all(OUTPUT_DIR) {
        eprintln!("Error creating output directory {}: {}", OUTPUT_DIR, e);
        std::process::exit(1);
    }

    let result = match cli.command {
        Commands::InstallEnv { env_name } => install_qiime2_amplicon_2024_10(&env_name),
        Commands::Demux { barcodes_file } => {
            let barcode_str = barcodes_file.to_str().unwrap_or_else(|| {
                eprintln!("Error: Invalid barcodes file path");
                std::process::exit(1);
            });
            demultiplex::run_demultiplex_combined(barcode_str).map_err(|e| e.into())
        }
        Commands::Pipeline {
            env_name,
            manifest,
            metadata,
            cores,
            target,
            skip_existing,
        } => {
            println!("Running QIIME2 pipeline with environment: {}", env_name);
            run_pipeline(&env_name, &manifest, &metadata, cores, &target, skip_existing)
        }
        Commands::RunAll {
            env_name,
            barcodes_file,
            manifest,
            metadata,
            cores,
            target,
            skip_existing,
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

            println!("==> Downloading database files if necessary...");
            if let Err(e) = download_databases(false) {
                eprintln!("Error downloading databases: {}", e);
                return;
            }

            println!("==> Running QIIME2 pipeline using manifest file: {}", manifest);
            run_pipeline(&env_name, &manifest, &metadata, cores, &target, skip_existing)
        }
        Commands::DownloadDBs { force } => {
            download_databases(force).unwrap();
            Ok(())
        }
    };

    // If any subcommand returned an Err, print it and exit with an error code.
    if let Err(e) = result {
        eprintln!("Application error: {}", e);
        std::process::exit(1);
    }
}
