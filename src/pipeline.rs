use std::process::{Command, Stdio};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::error::Error;
use std::time::Duration;
use std::collections::HashMap;

use indicatif::{ProgressBar, ProgressStyle};
use flate2::read::GzDecoder;
use reqwest;
use csv::{ReaderBuilder, WriterBuilder};

use crate::logger::log_action;
use crate::color_print::{print_info, print_error, print_success};
use crate::{OUTPUT_DIR};

// We'll assume we can get the verbose bool from a function.
fn verbose_mode() -> bool {
    // In real code, you'd reference the AtomicBool in main.rs
    // For example:
    // crate::main::VERBOSE_MODE.load(std::sync::atomic::Ordering::Relaxed)
    super::VERBOSE_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Helper to generate an output file/folder path within OUTPUT_DIR.
fn out_path(relative: &str) -> String {
    format!("{}/{}", OUTPUT_DIR, relative)
}

/// Wraps an operation `f` in a spinner-based progress bar if not in verbose mode.
fn run_step<F>(description: &str, f: F) -> Result<(), Box<dyn Error>>
where
    F: FnOnce() -> Result<(), Box<dyn Error>>,
{
    log_action(&format!("Starting step: {}", description));

    // If verbose, just print the step description and run it
    if verbose_mode() {
        print_info(&format!("==> {}", description));
        let result = f();
        match &result {
            Ok(_) => print_success(&format!("{} ✔", description)),
            Err(_) => print_error(&format!("{} ✘", description)),
        }
        return result;
    }

    // Otherwise, create a spinner progress bar
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
    pb.enable_steady_tick(Duration::from_millis(100));
    pb.set_message(description.to_owned());

    let result = f();
    match &result {
        Ok(_) => {
            pb.finish_with_message(format!("{} ✔", description));
            log_action(&format!("Step succeeded: {}", description));
        },
        Err(_) => {
            pb.abandon_with_message(format!("{} ✘", description));
            log_action(&format!("Step failed: {}", description));
        }
    }
    result
}

/// Checks if a specified conda environment already exists.
pub fn conda_env_exists(env_name: &str) -> Result<bool, Box<dyn Error>> {
    let output = Command::new("conda")
        .arg("env")
        .arg("list")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            print_error(&format!("Failed to run 'conda env list': {}", e));
            return Err(e.into());
        }
    };

    if !output.status.success() {
        let msg = "Could not retrieve conda environment list.";
        print_error(msg);
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

/// Installs the specified QIIME2 environment if it doesn't already exist.
pub fn install_qiime2_amplicon_2024_10(env_name: &str) -> Result<(), Box<dyn Error>> {
    match conda_env_exists(env_name) {
        Ok(true) => {
            print_info(&format!("Conda environment '{}' already exists. Skipping creation.", env_name));
            return Ok(());
        }
        Ok(false) => {
            print_info(&format!("Installing environment '{}'", env_name));
        }
        Err(e) => {
            print_error(&format!("Error checking conda environments: {}", e));
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
        vec![format!(
            "conda env create -n {} --file https://data.qiime2.org/distro/amplicon/qiime2-amplicon-2024.10-py310-linux-conda.yml",
            env_name
        )]
    } else {
        vec!["echo 'Unknown or unsupported platform'".to_string()]
    };

    for cmd in &commands {
        run_shell_command(cmd)?;
    }
    print_success(&format!(
        "Installation complete. You can activate via: conda activate {}",
        env_name
    ));
    Ok(())
}

/// Executes a shell command (via `bash -c`) in either quiet or verbose mode.
fn run_shell_command(cmd: &str) -> Result<(), Box<dyn Error>> {
    log_action(&format!("Running shell command: {}", cmd));
    if verbose_mode() {
        println!("[CMD] {}", cmd);
    }

    let (stdout_setting, stderr_setting) = if verbose_mode() {
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
        .status()?;

    if !status.success() {
        let msg = format!("Command failed: {}", cmd);
        print_error(&msg);
        return Err(msg.into());
    }
    Ok(())
}

/// Runs a QIIME command in a specified conda environment via `conda run`.
fn run_conda_qiime_command(env: &str, qiime_args: &str) -> Result<(), Box<dyn Error>> {
    log_action(&format!("Running QIIME command in {}: qiime {}", env, qiime_args));
    if verbose_mode() {
        println!("[QIIME CMD] qiime {}", qiime_args);
    }
    let mut args: Vec<&str> = vec!["run", "-n", env, "qiime"];
    args.extend(qiime_args.split_whitespace());

    let (stdout_setting, stderr_setting) = if verbose_mode() {
        (Stdio::inherit(), Stdio::inherit())
    } else {
        (Stdio::null(), Stdio::null())
    };

    let status = Command::new("conda")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(stdout_setting)
        .stderr(stderr_setting)
        .status()?;

    if !status.success() {
        let msg = format!("QIIME command failed: qiime {}", qiime_args);
        print_error(&msg);
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
        print_info(&format!(
            "File '{}' already exists, skipping download.",
            output_path
        ));
        return Ok(());
    }
    print_info(&format!("Downloading '{}' to '{}'...", url, output_path));
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
        print_info(&format!(
            "File '{}' already exists, skipping unzip.",
            output_path
        ));
        return Ok(());
    }
    print_info(&format!("Unzipping '{}' to '{}'...", input_path, output_path));
    let input_file = File::open(input_path)?;
    let mut gz = GzDecoder::new(input_file);
    let mut out = File::create(output_path)?;
    io::copy(&mut gz, &mut out)?;
    Ok(())
}

/// Downloads (and unzips) the required database files into `OUTPUT_DIR/db/pr2`.
pub fn download_databases(force: bool) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(out_path("db/pr2"))?;

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

    print_success("Database download and extraction complete.");
    Ok(())
}

/// Primary pipeline function: runs Steps 2–7 of the QIIME2 workflow.
pub fn run_pipeline(
    env_name: &str,
    manifest: &str,
    metadata: &str,
    cores: usize,
    target: &str,
    skip_existing: bool,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(OUTPUT_DIR)?;

    // Adapter/primer sequences
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
            print_error(&format!("Unsupported target: {}. Use '16s' or '18s'.", other));
            return Err("Unsupported target".into());
        }
    };

    // Step 2: Import Files
    let pe_demux_qza = out_path("paired-end-demux.qza");
    if skip_existing && Path::new(&pe_demux_qza).exists() {
        print_info(&format!("Skipping import ({} exists).", pe_demux_qza));
    } else {
        run_step("Importing files with manifest", || {
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

    // Summarize
    let pe_demux_qzv = out_path("paired-end-demux.qzv");
    if skip_existing && Path::new(&pe_demux_qzv).exists() {
        print_info(&format!("Skipping demux summarize ({} exists).", pe_demux_qzv));
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

    // Step 3: Trim Reads (Cutadapt)
    let pe_trimmed_qza = out_path("paired-end-demux-trimmed.qza");
    let pe_trimmed_qzv = out_path("paired-end-demux-trimmed.qzv");
    if skip_existing && Path::new(&pe_trimmed_qza).exists() && Path::new(&pe_trimmed_qzv).exists() {
        print_info(&format!("Skipping Cutadapt ({} exists).", pe_trimmed_qza));
    } else {
        run_step("Trimming reads with Cutadapt", || {
            let cutadapt_cmd = format!(
                "cutadapt trim-paired --i-demultiplexed-sequences {}  \
                 --p-cores {} --p-adapter-f {} --p-adapter-r {} \
                 --p-error-rate 0.1 --p-overlap 3 --verbose \
                 --o-trimmed-sequences {}",
                pe_demux_qza, cores, adapter_f, adapter_r, pe_trimmed_qza
            );
            run_conda_qiime_command(env_name, &cutadapt_cmd)
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
        print_info("Skipping DADA2 (existing outputs).");
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
        
        // Add feature table summarization with metadata
        let table_dada2_qzv = out_path("asvs/table-dada2.qzv");
        run_step("Summarizing feature table with metadata", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-table summarize \
                 --i-table {} \
                 --o-visualization {} \
                 --m-sample-metadata-file {}",
                table_dada2_qza, table_dada2_qzv, metadata
            ))
        })?;
    }

    // Step 5: Export Denoised Data
    let asv_table_dir = out_path("asv_table");
    run_step("Exporting ASV table", || {
        if skip_existing && Path::new(&format!("{}/feature-table.biom", asv_table_dir)).exists() {
            print_info("Skipping export of ASV table (feature-table.biom exists).");
            return Ok(());
        }
        run_conda_qiime_command(env_name, &format!(
            "tools export --input-path {} --output-path {}",
            table_dada2_qza, asv_table_dir
        ))
    })?;
    run_step("Converting BIOM to TSV", || {
        let biom_path = format!("{}/feature-table.biom", asv_table_dir);
        let tsv_path = format!("{}/asv-table.tsv", asv_table_dir);
        if skip_existing && Path::new(&tsv_path).exists() {
            print_info("Skipping BIOM-to-TSV conversion (asv-table.tsv exists).");
            return Ok(());
        }
        convert_biom_to_tsv_conda(env_name, &biom_path, &tsv_path)
    })?;
    run_step("Exporting representative sequences", || {
        let rep_seqs_export_dir = out_path("asvs");
        if skip_existing && Path::new(&format!("{}/dna-sequences.fasta", rep_seqs_export_dir)).exists() {
            print_info("Skipping export rep-seqs (dna-sequences.fasta exists).");
            return Ok(());
        }
        run_conda_qiime_command(env_name, &format!(
            "tools export --input-path {} --output-path {}",
            rep_seqs_dada2_qza, rep_seqs_export_dir
        ))
    })?;
    let rep_seqs_dada2_qzv = out_path("asvs/rep-seqs-dada2.qzv");
    if !skip_existing || !Path::new(&rep_seqs_dada2_qzv).exists() {
        run_step("Tabulating representative sequences", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-table tabulate-seqs --i-data {} --o-visualization {}",
                rep_seqs_dada2_qza, rep_seqs_dada2_qzv
            ))
        })?;
    }
    let table_dada2_qzv = out_path("asvs/table-dada2.qzv");
    if !skip_existing || !Path::new(&table_dada2_qzv).exists() {
        run_step("Summarizing feature table", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-table summarize --i-table {} --o-visualization {}",
                table_dada2_qza, table_dada2_qzv
            ))
        })?;
    }

    // Step 6: Taxonomic Annotation
    let pr2_dir = out_path("db/pr2");
    let pr2_qza = out_path("db/pr2/pr2.qza");
    if !skip_existing || !Path::new(&pr2_qza).exists() {
        run_step("Importing pr2 sequences", || {
            fs::create_dir_all(&pr2_dir)?;
            run_conda_qiime_command(env_name, &format!(
                "tools import --type FeatureData[Sequence] \
                 --input-path {} \
                 --output-path {}",
                out_path("db/pr2/pr2_with_taxonomy_simple.fasta"),
                pr2_qza
            ))
        })?;
    }

    let pr2_tax_qza = out_path("db/pr2/pr2_tax.qza");
    if !skip_existing || !Path::new(&pr2_tax_qza).exists() {
        run_step("Importing pr2 taxonomy", || {
            run_conda_qiime_command(env_name, &format!(
                "tools import --type FeatureData[Taxonomy] \
                 --input-format HeaderlessTSVTaxonomyFormat \
                 --input-path {} \
                 --output-path {}",
                out_path("db/pr2/pr2_taxonomy.tsv"),
                pr2_tax_qza
            ))
        })?;
    }

    let pr2_extracts_qza = out_path("db/pr2/pr2_extracts.qza");
    if !skip_existing || !Path::new(&pr2_extracts_qza).exists() {
        run_step("Extracting pr2 reads", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-classifier extract-reads \
                 --i-sequences {} \
                 --p-f-primer {} \
                 --p-r-primer {} \
                 --o-reads {}",
                pr2_qza, primer_f, primer_r, pr2_extracts_qza
            ))
        })?;
    }

    let pr2_classifier_qza = out_path("db/pr2/pr2_classifier.qza");
    if !skip_existing || !Path::new(&pr2_classifier_qza).exists() {
        run_step("Fitting pr2 classifier", || {
            run_conda_qiime_command(env_name, &format!(
                "feature-classifier fit-classifier-naive-bayes \
                 --i-reference-reads {} \
                 --i-reference-taxonomy {} \
                 --o-classifier {} \
                 --p-classify--chunk-size 100000",
                pr2_extracts_qza, pr2_tax_qza, pr2_classifier_qza
            ))
        })?;
    }

    let pr2_tax_sklearn_qza = out_path("pr2_tax_sklearn.qza");
    if !skip_existing || !Path::new(&pr2_tax_sklearn_qza).exists() {
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

    let pr2_tax_sklearn_qzv = out_path("pr2_tax_sklearn.qzv");
    if !skip_existing || !Path::new(&pr2_tax_sklearn_qzv).exists() {
        run_step("Tabulating classified taxonomy", || {
            run_conda_qiime_command(env_name, &format!(
                "metadata tabulate --m-input-file {} --o-visualization {}",
                pr2_tax_sklearn_qza, pr2_tax_sklearn_qzv
            ))
        })?;
    }

    let asv_tax_dir = out_path("asv_tax_dir");
    if !skip_existing || !Path::new(&format!("{}/taxonomy.tsv", asv_tax_dir)).exists() {
        run_step("Exporting pr2 taxonomy", || {
            run_conda_qiime_command(env_name, &format!(
                "tools export --input-path {} --output-path {}",
                pr2_tax_sklearn_qza, asv_tax_dir
            ))
        })?;
        run_step("Renaming pr2 taxonomy file", || {
            let pr2_taxonomy_tsv = format!("{}/pr2_taxonomy.tsv", asv_tax_dir);
            let old_tsv = format!("{}/taxonomy.tsv", asv_tax_dir);
            let mv_cmd = format!("mv {} {}", old_tsv, pr2_taxonomy_tsv);
            run_shell_command(&mv_cmd)
        })?;
    }

    // Step 7: Merge ASV Table with Taxonomy
    let merged_output = out_path("asv_count_tax.tsv");
    if skip_existing && Path::new(&merged_output).exists() {
        print_info(&format!("Skipping merge ({} exists).", merged_output));
    } else {
        run_step("Merging ASV and taxonomy tables", merge_asv_taxonomy)?;
    }

    print_success("Pipeline completed successfully!");
    print_info("Final summary: see 'windchime_out/asv_count_tax.tsv' for merged results.");

    if Path::new(&out_path("asvs/stats-dada2.qzv")).exists() {
        print_info("You can view 'asvs/stats-dada2.qzv' in QIIME2 View for DADA2 stats.");
    }

    Ok(())
}

/// Merges the ASV count table with the assigned taxonomy, producing `asv_count_tax.tsv`.
fn merge_asv_taxonomy() -> Result<(), Box<dyn Error>> {
    use std::collections::HashMap;
    use std::io;

    // Read the ASV table
    let asv_table_path = out_path("asv_table/asv-table.tsv");
    let mut asv_reader = ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .comment(Some(b'#'))
        .from_path(&asv_table_path)?;

    let asv_headers = asv_reader.headers()?.clone();
    let mut asv_map: HashMap<String, Vec<String>> = HashMap::new();
    for record in asv_reader.records() {
        let rec = record?;
        let feature_id = rec.get(0).unwrap_or("").to_string();
        asv_map.insert(feature_id, rec.iter().map(|s| s.to_string()).collect());
    }

    // Read the pr2 taxonomy table
    let pr2_tax_path = out_path("asv_tax_dir/pr2_taxonomy.tsv");
    let mut pr2_reader = ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .from_path(&pr2_tax_path)?;

    let pr2_headers = pr2_reader.headers()?.clone();
    let mut pr2_map: HashMap<String, Vec<String>> = HashMap::new();
    for record in pr2_reader.records() {
        let rec = record?;
        let feature_id = rec.get(0).unwrap_or("").to_string();
        pr2_map.insert(feature_id, rec.iter().map(|s| s.to_string()).collect());
    }

    // Write merged
    let merged_path = out_path("asv_count_tax.tsv");
    let mut wtr = WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(&merged_path)?;

    // Build merged header
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
            continue;
        }
        merged_header.push(format!("pr2_{}", col));
    }
    wtr.write_record(&merged_header)?;

    // Merge rows
    for (feature_id, asv_record) in asv_map.iter() {
        let mut merged_record = asv_record.clone();
        if let Some(pr2_record) = pr2_map.get(feature_id) {
            // skip the first column from pr2
            merged_record.extend(pr2_record.iter().skip(1).cloned());
        } else {
            for _ in 1..pr2_headers.len() {
                merged_record.push(String::new());
            }
        }
        wtr.write_record(&merged_record)?;
    }
    wtr.flush()?;

    print_success(&format!(
        "Merged ASV count and taxonomy table written to {}",
        merged_path
    ));
    Ok(())
}
