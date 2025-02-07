# Windchime

**Windchime** is a Rust-based command-line interface (CLI) tool designed to simplify running a [QIIME2](https://qiime2.org/) amplicon sequencing pipeline for 16S/18S analysis. It provides integrated steps for environment setup, demultiplexing, database preparation, and executing a multi-step QIIME2 workflow—from importing files to generating merged output tables.

> **Note:** This tool assumes that you have Conda installed and available in your system's PATH, as it leverages Conda for installing QIIME2 environments and running QIIME2 commands.

## Requirements

- **Rust:** Ensure you have [Rust installed](https://www.rust-lang.org/tools/install) to build from source.
- **Conda:** Required for managing the QIIME2 environment. [Install Miniconda or Anaconda](https://docs.conda.io/en/latest/miniconda.html) if you haven't already.
- **QIIME2:** The pipeline depends on QIIME2 commands, which are executed within a Conda environment (windchime can install this for you).
- **Internet Connection:** Needed to download database files and QIIME2 environment YAML files.

## Installation

You can install windchime using our installer script:

```bash
curl -fsSL https://raw.githubusercontent.com/nikothomas/windchime/main/install.sh | sh
```

This will:
1. Download the latest release for your platform:
   - Linux (x86_64)
   - macOS (Intel x86_64 or Apple Silicon ARM64)
2. Install it to `~/.windchime/bin`
3. Add it to your PATH

Alternatively, you can:
1. Download the latest release manually from the [releases page](https://github.com/nik/windchime/releases)
2. Extract it and place it in a directory in your PATH

> **Note:** For other platforms (e.g., Linux ARM64), you'll need to build from source using `cargo build --release`.

## Usage

Windchime is organized into several subcommands, each covering a different part of the workflow. You can enable verbose output with the `-v` or `--verbose` flag to see full command details instead of spinners.

### Global Options

- `-v, --verbose`  
  Enable verbose output. When active, the tool prints the full QIIME commands executed.

### Subcommands

#### 1. InstallEnv

Install (or skip if already present) the specified QIIME2 Conda environment.

```bash
windchime install-env [OPTIONS]
```

**Options:**

- `-e, --env-name <env_name>`  
  Name of the QIIME2 environment to install.  
  *Default:* `qiime2-amplicon-2024.10`

**Example:**

```bash
windchime install-env --env-name qiime2-amplicon-2024.10
```

#### 2. Demux

Run demultiplexing using a barcodes file. This subcommand leverages the internal `demultiplex` module.

```bash
windchime demux <barcodes_file>
```

**Example:**

```bash
windchime demux barcodes.tsv
```

#### 3. Pipeline

Execute steps 2–7 of the QIIME2 pipeline using a QIIME2 manifest file. This command covers import, trimming, denoising, taxonomic classification, and merging outputs.

```bash
windchime pipeline [OPTIONS]
```

**Options:**

- `-e, --env-name <env_name>`  
  QIIME2 environment name.  
  *Default:* `qiime2-amplicon-2024.10`
- `-m, --manifest <manifest>`  
  Path to the QIIME2 manifest file.  
  *Default:* `manifest.tsv`
- `--cores <cores>`  
  Number of CPU cores to use.  
  *Default:* `1`
- `-t, --target <target>`  
  Target region: either `16s` or `18s`.  
  *Default:* `18s`
- `--skip-existing`  
  If set, skips any pipeline steps where expected output files already exist.
- `--use-pretrained-classifier`  
  Use a pre-trained classifier instead of training from PR2 references.  
  *Default:* `true`

**Example:**

```bash
windchime pipeline --env-name qiime2-amplicon-2024.10 \
                   --manifest manifest.tsv \
                   --cores 4 \
                   --target 16s \
                   --skip-existing
```

#### 4. RunAll

A single command to run the entire workflow: install the environment (if needed), demultiplex, generate the manifest, download databases, and execute the pipeline.

```bash
windchime runall [OPTIONS]
```

**Options:**

- `-e, --env-name <env_name>`  
  QIIME2 environment name.  
  *Default:* `qiime2-amplicon-2024.10`
- `--barcodes-file <barcodes_file>`  
  Path to the barcodes file for demultiplexing.  
  *Default:* `barcodes.tsv`
- `-m, --manifest <manifest>`  
  Path for the QIIME2 manifest file.  
  *Default:* `manifest.tsv`
- `--cores <cores>`  
  Number of CPU cores to use.  
  *Default:* `1`
- `-t, --target <target>`  
  Target region, either `16s` or `18s`.  
  *Default:* `18s`
- `--skip-existing`  
  Skip steps if expected outputs already exist.
- `--use-pretrained-classifier`  
  Use a pre-trained classifier instead of training from PR2 references.  
  *Default:* `true`

**Example:**

```bash
windchime runall --env-name qiime2-amplicon-2024.10 \
                  --barcodes-file barcodes.tsv \
                  --manifest manifest.tsv \
                  --cores 4 \
                  --target 18s \
                  --skip-existing
```

#### 5. DownloadDBs

Download (and unzip) the required pr2 database files to `windchime_out/db/pr2`. Use the force option to re-download even if the files already exist.

```bash
windchime downloaddbs [OPTIONS]
```

**Options:**

- `-f, --force`  
  Force re-download and unzip even if the database files are present.  
  *Default:* `false`

**Example:**

```bash
windchime downloaddbs --force
```

## Pipeline Overview

Windchime's pipeline integrates several QIIME2 steps, which are executed in order:

1. **Importing Files:**  
   Uses a manifest file to import paired-end sequencing data into a QIIME2 artifact.
2. **Validation & Summarization:**  
   Validates the imported data and creates summary visualizations.
3. **Trimming Reads:**  
   Uses Cutadapt to remove adapter/primer sequences.
4. **Denoising with DADA2:**  
   Performs error correction and generates Amplicon Sequence Variants (ASVs) using the `dada2 denoise-paired` command.
5. **Exporting Data:**  
   Exports the ASV table (BIOM format) and converts it to TSV; exports representative sequences.
6. **Taxonomic Annotation:**  
   Downloads and imports the pr2 database, extracts reads using target-specific primers, fits a classifier, and classifies sequences.
7. **Merging Tables:**  
   Merges the ASV count table with the taxonomic assignments into a single TSV output (`asv_count_tax.tsv`).

All generated files are stored in the `windchime_out` directory.

## Verbose Mode

For detailed debugging information, use the `--verbose` (or `-v`) flag. In verbose mode, Windchime prints the exact QIIME2 and shell commands being executed rather than displaying progress spinners.

**Example:**

```bash
windchime pipeline --verbose --env-name qiime2-amplicon-2024.10 --manifest manifest.tsv
```

## Attribution

Windchime borrows significantly from the original QIIME2 ASV protocols developed by the Allen Lab at the Scripps Institution of Oceanography:

- **QIIME2 18S v9 ASV Protocol**  
  [https://github.com/allenlab/QIIME2_18Sv9_ASV_protocol](https://github.com/allenlab/QIIME2_18Sv9_ASV_protocol)

- **QIIME2 16S ASV Protocol**  
  [https://github.com/allenlab/QIIME2_16S_ASV_protocol](https://github.com/allenlab/QIIME2_16S_ASV_protocol)

## Contributing

Contributions are welcome! If you find any bugs or have feature suggestions, please open an issue or submit a pull request on the project repository.

## License

This project is licensed under the [MIT License](LICENSE).

## Contact

For further questions or feedback, please reach out to:

- **Author:** Nikolas Yanek-Chrones
- **Email:** research@icarai.io


