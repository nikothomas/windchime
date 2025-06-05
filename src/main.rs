mod demultiplex;
mod pipeline;
mod wizard;
mod config;
mod color_print;
mod logger;

use clap::{Parser, Subcommand};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::fs;

use config::WindchimeConfig;
use logger::{init_log, log_action};
use color_print::{print_info, print_success, print_error};

/// GLOBAL VERBOSE FLAG: true = print commands verbosely, false = use progress bars.
static VERBOSE_MODE: AtomicBool = AtomicBool::new(false);

/// OUTPUT DIRECTORY for all generated files.
pub const OUTPUT_DIR: &str = "windchime_out";

/// CLI definition using Clap.
#[derive(Parser, Debug)]
#[command(name = "windchime", about = "A Rust CLI for QIIME2 16S/18S pipeline", version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Enable verbose output: print the full QIIME command for each step
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Optional path to a config file (TOML). If provided, default settings are loaded from there.
    #[arg(long)]
    config: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Install (or skip if existing) the specified QIIME2 environment.
    InstallEnv {
        /// Name of the conda environment to install
        #[arg(short, long, default_value = "qiime2-amplicon-2024.10")]
        env_name: String,
    },
    /// Run demultiplexing using a barcodes file.
    Demux {
        /// Path to the barcodes file for demultiplexing.
        barcodes_file: String,

        /// Whether to skip if demultiplexed output already exists
        #[arg(long, default_value_t = false)]
        skip_existing: bool,
    },
    /// Execute only Steps 2–7 of the pipeline, optionally skipping existing outputs.
    Pipeline {
        #[arg(short, long, default_value = "qiime2-amplicon-2024.10")]
        env_name: String,

        /// QIIME2 manifest file.
        #[arg(short, long, default_value = "manifest.tsv")]
        manifest: String,

        /// Number of CPU cores to use.
        #[arg(long, default_value_t = 1)]
        cores: usize,

        /// Target region (16s or 18s).
        #[arg(short, long, default_value = "18s")]
        target: String,

        /// Skip pipeline steps if expected outputs already exist.
        #[arg(long, default_value_t = false)]
        skip_existing: bool,

        /// Use a pre-trained classifier instead of training from PR2 references.
        #[arg(long, default_value_t = true)]
        use_pretrained_classifier: bool,
    },
    /// Single command: install env if needed, demultiplex, generate manifest, download DBs, pipeline
    RunAll {
        #[arg(short, long, default_value = "qiime2-amplicon-2024.10")]
        env_name: String,

        /// Path to the barcodes file for demultiplexing.
        #[arg(long, default_value = "barcodes.tsv")]
        barcodes_file: String,

        /// QIIME2 manifest file.
        #[arg(short, long, default_value = "manifest.tsv")]
        manifest: String,

        /// Number of CPU cores to use.
        #[arg(long, default_value_t = 1)]
        cores: usize,

        /// Target region (16s or 18s).
        #[arg(short, long, default_value = "18s")]
        target: String,

        /// Skip pipeline steps if expected outputs already exist.
        #[arg(long, default_value_t = false)]
        skip_existing: bool,

        /// Use a pre-trained classifier instead of training from PR2 references.
        #[arg(long, default_value_t = true)]
        use_pretrained_classifier: bool,
    },
    /// Download the database files (and unzip them if needed).
    DownloadDBs {
        /// Force re-download and unzip even if the files already exist.
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
    /// Interactive wizard that guides you through environment setup, demux, etc.
    Wizard,
    /// Info subcommand: show environment availability, OS details, config, etc.
    Info,
}

fn main() {
    let cli = Cli::parse();

    // Initialize logging to windchime.log
    init_log();

    // Load config file if provided
    let mut config_data = WindchimeConfig::default();
    if let Some(cfg_path) = &cli.config {
        match config::load_config(cfg_path) {
            Ok(cfg) => config_data = cfg,
            Err(e) => {
                print_error(&format!("Failed to load config file {}: {}", cfg_path, e));
            }
        }
    }

    // Set the global verbose flag
    VERBOSE_MODE.store(cli.verbose, Ordering::Relaxed);

    // Ensure the output directory exists
    if let Err(e) = fs::create_dir_all(OUTPUT_DIR) {
        print_error(&format!("Error creating output directory {}: {}", OUTPUT_DIR, e));
        process::exit(1);
    }

    // Log the action and parse subcommands
    log_action(&format!("Starting Windchime with command: {:?}", cli.command));

    let result = match cli.command {
        Commands::InstallEnv { env_name } => {
            pipeline::install_qiime2_amplicon_2024_10(&env_name)
        }
        Commands::Demux {
            barcodes_file,
            skip_existing,
        } => {
            print_info("Running demultiplex step...");
            demultiplex::run_demultiplex_combined(&barcodes_file, skip_existing)
                .map_err(|e| e.into())
        }
        Commands::Pipeline {
            env_name,
            manifest,
            cores,
            target,
            skip_existing,
            use_pretrained_classifier,
        } => {
            print_info(&format!("Running QIIME2 pipeline with environment: {}", env_name));
            pipeline::run_pipeline(
                &env_name,
                &manifest,
                cores,
                &target,
                skip_existing,
                use_pretrained_classifier,
                219,
                194,
            )
        }
        Commands::RunAll {
            env_name,
            barcodes_file,
            manifest,
            cores,
            target,
            skip_existing,
            use_pretrained_classifier,
        } => {
            print_info(&format!("==> Checking conda environment '{}'", env_name));
            pipeline::install_qiime2_amplicon_2024_10(&env_name).unwrap();

            print_info("==> Running demultiplexing step...");
            demultiplex::run_demultiplex_combined(&barcodes_file, skip_existing).unwrap();

            print_info("==> Generating QIIME2 manifest file...");
            demultiplex::generate_qiime_manifest(&barcodes_file, &manifest).unwrap();

            print_info("==> Downloading database files if necessary...");
            pipeline::download_databases(false).unwrap();

            print_info(&format!("==> Running QIIME2 pipeline using manifest file: {}", manifest));
            pipeline::run_pipeline(
                &env_name,
                &manifest,
                cores,
                &target,
                skip_existing,
                use_pretrained_classifier,
                219,
                194,
            )
        }
        Commands::DownloadDBs { force } => {
            pipeline::download_databases(force)
        }
        Commands::Wizard => {
            wizard::run_wizard()
        }
        Commands::Info => {
            print_info("Gathering system and environment info...");
            // Show version
            print_success(&format!("Windchime version: {}", env!("CARGO_PKG_VERSION")));
            
            // Show OS details
            let os = std::env::consts::OS;
            let arch = std::env::consts::ARCH;
            print_success(&format!("OS: {}, ARCH: {}", os, arch));

            // Check conda presence
            match pipeline::conda_env_exists("base") {
                Ok(_) => print_success("Conda appears to be installed and accessible."),
                Err(e) => print_error(&format!("Conda not found or error: {}", e)),
            }

            // Print local config (this is just an example)
            print_info("Loaded config:");
            print_info(&format!("{:#?}", config_data));

            Ok(())
        }
    };

    if let Err(e) = result {
        print_error(&format!("Application error: {}", e));
        process::exit(1);
    }

    log_action("Windchime finished successfully.");
    print_success("All done!");
}
