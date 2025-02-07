use dialoguer::{theme::ColorfulTheme, Input, Confirm};
use std::error::Error;
use crate::{pipeline, demultiplex, OUTPUT_DIR};
use crate::color_print::{print_info, print_success, print_error};
use std::fs;

/// Example interactive wizard that prompts the user for typical pipeline steps.
pub fn run_wizard() -> Result<(), Box<dyn Error>> {
    print_info("Welcome to the Windchime Wizard!");

    // Prompt for environment name
    let env_name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter the QIIME2 environment name (default: qiime2-amplicon-2024.10)")
        .default("qiime2-amplicon-2024.10".into())
        .interact_text()?;

    // Install environment?
    let install_env = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Would you like to install/check this environment now?")
        .default(true)
        .interact()?;
    if install_env {
        pipeline::install_qiime2_amplicon_2024_10(&env_name)?;
        print_success(&format!("Environment '{}' is ready.", env_name));
    }

    // Prompt for barcodes file
    let barcodes_file: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Path to barcodes file (or leave blank to skip demux)")
        .default("".into())
        .allow_empty(true)
        .interact_text()?;

    let do_demux = !barcodes_file.trim().is_empty();
    if do_demux {
        print_info("Running demultiplex step...");
        demultiplex::run_demultiplex_combined(&barcodes_file, false)?;
        // Generate manifest
        let generate_manifest = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Generate QIIME manifest from the barcodes file?")
            .default(true)
            .interact()?;
        if generate_manifest {
            demultiplex::generate_qiime_manifest(&barcodes_file, "manifest.tsv")?;
            print_success("Manifest file created in output directory.");
        }
    }

    // Download DBs?
    let download_dbs = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Download reference databases now?")
        .default(true)
        .interact()?;
    if download_dbs {
        pipeline::download_databases(false)?;
        print_success("Reference databases downloaded!");
    }

    // Prompt for pipeline steps
    let run_pipeline_now = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Run the QIIME pipeline now?")
        .default(true)
        .interact()?;
    if run_pipeline_now {
        // Collect pipeline arguments
        let manifest = if do_demux { "manifest.tsv".to_string() } else {
            Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter the manifest file path")
                .default("manifest.tsv".into())
                .interact_text()?
        };

        let cores: usize = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Number of CPU cores to use")
            .default("1".into())
            .validate_with(|input: &String| -> Result<(), &str> {
                match input.parse::<usize>() {
                    Ok(_) => Ok(()),
                    Err(_) => Err("Please enter a positive integer"),
                }
            })
            .interact_text()?
            .parse()?;

        let target: String = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Target region (16s/18s)")
            .default("18s".into())
            .validate_with(|input: &String| -> Result<(), &str> {
                let lower = input.to_lowercase();
                if lower == "16s" || lower == "18s" {
                    Ok(())
                } else {
                    Err("Must be '16s' or '18s'")
                }
            })
            .interact_text()?;

        // Run pipeline
        pipeline::run_pipeline(&env_name, &manifest, "metadata.tsv", cores, &target, false)?;
        print_success("Pipeline completed!");
    }

    // Done
    print_success("Wizard completed successfully!");
    Ok(())
}
