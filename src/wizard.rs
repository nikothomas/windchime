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

    // Prompt for barcodes file (optional)
    let barcodes_file: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Path to barcodes file (leave blank to skip demultiplexing)")
        .default("".into())
        .allow_empty(true)
        .interact_text()?;

    let do_demux = !barcodes_file.trim().is_empty();
    if do_demux {
        print_info("Running demultiplex step...");
        demultiplex::run_demultiplex_combined(&barcodes_file, false)?;
        print_success("Demultiplexing complete.");

        // Generate manifest?
        let generate_manifest = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Generate QIIME manifest from the barcodes file?")
            .default(true)
            .interact()?;
        if generate_manifest {
            demultiplex::generate_qiime_manifest(&barcodes_file, "manifest.tsv")?;
            print_success("Manifest file created in output directory (manifest.tsv).");
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

    // Prompt if user wants to run the full pipeline
    let run_pipeline_now = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Run the QIIME pipeline now?")
        .default(true)
        .interact()?;
    if run_pipeline_now {
        // Collect pipeline arguments
        let manifest = if do_demux {
            // If we've demultiplexed and created a manifest already, default to 'manifest.tsv'
            "manifest.tsv".to_string()
        } else {
            // Otherwise, let the user specify a manifest
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
            .with_prompt("Target region (16s/18sv4/18sv9)")
            .default("18sv9".into())
            .validate_with(|input: &String| -> Result<(), &str> {
                let lower = input.to_lowercase();
                if lower == "16s" || lower == "18sv4" || lower == "18sv9" || lower == "18s" {
                    Ok(())
                } else {
                    Err("Must be '16s', '18sv4', or '18sv9' (or '18s' for backward compatibility with 18sv9)")
                }
            })
            .interact_text()?;

        // Truncation lengths for DADA2 - set defaults based on target region
        let (default_trunc_f, default_trunc_r) = match target.to_lowercase().as_str() {
            "16s" => ("219", "194"),
            "18sv4" => ("262", "223"),
            "18sv9" | "18s" => ("123", "91"),
            _ => ("219", "194"), // fallback to 16s defaults
        };

        let trunc_len_f: usize = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Forward read trunc length for DADA2 (0 = no truncation)")
            .default(default_trunc_f.into())
            .validate_with(|input: &String| -> Result<(), &str> {
                match input.parse::<usize>() {
                    Ok(_) => Ok(()),
                    Err(_) => Err("Please enter a non-negative integer"),
                }
            })
            .interact_text()?
            .parse()?;

        let trunc_len_r: usize = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Reverse read trunc length for DADA2 (0 = no truncation)")
            .default(default_trunc_r.into())
            .validate_with(|input: &String| -> Result<(), &str> {
                match input.parse::<usize>() {
                    Ok(_) => Ok(()),
                    Err(_) => Err("Please enter a non-negative integer"),
                }
            })
            .interact_text()?
            .parse()?;

        // Ask if we should skip artifacts already present
        let skip_existing = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Skip existing QIIME artifacts if found?")
            .default(false)
            .interact()?;

        // Ask if we should use a pre-trained classifier
        let use_pretrained_classifier = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Use a pre-trained classifier (downloaded) instead of training from PR2 references?")
            .default(true)
            .interact()?;

        // Run pipeline
        print_info("Launching pipeline...");
        pipeline::run_pipeline(
            &env_name,
            &manifest,
            cores,
            &target,
            skip_existing,
            use_pretrained_classifier,
            trunc_len_f,
            trunc_len_r
        )?;
        print_success("Pipeline completed!");
    }

    // Done
    print_success("Wizard completed successfully!");
    Ok(())
}
