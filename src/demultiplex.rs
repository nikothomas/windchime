use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bio::io::fastq;
use flate2::{read::MultiGzDecoder, write::GzEncoder, Compression};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

/// Directory where output files are written.
const OUTPUT_DIR: &str = "windchime_out";

/// Simple helper for constructing an output path (as a `String`).
fn out_path(filename: &str) -> String {
    format!("{}/{}", OUTPUT_DIR, filename)
}

/// Runs the demultiplexing logic using the provided barcodes file.
///
/// # Assumptions
///
/// - The `barcodes_file` is a tab-separated file with six columns:
///   1) `name`
///   2) `file_name`
///   3) `idx1`
///   4) `seq1`
///   5) `idx2`
///   6) `seq2`
/// - The first line is a header and will be skipped.
/// - This function will look for `"{file_name}_R1_001.fastq.gz"`, and if not found,
///   it will look for `"{file_name}_R1_001.fastq"`. The same logic applies for R2.
/// - The output file names are constructed as `"{name}_{seq2}_L001_R1_001.fastq.gz"` (and `_R2_`).
///
/// # Errors
///
/// Returns an `io::Error` if:
/// - The barcodes file cannot be read.
/// - Any FASTQ files cannot be opened.
/// - Writing to output files fails.
pub fn run_demultiplex_combined(barcodes_file: &str) -> io::Result<()> {
    // Open the barcodes file
    let file = File::open(barcodes_file).map_err(|e| {
        eprintln!("Unable to open barcodes file '{}': {}", barcodes_file, e);
        e
    })?;
    let reader = BufReader::new(file);

    // Read all lines (skipping the header)
    let barcode_lines: Vec<_> = reader
        .lines()
        .enumerate()
        .filter_map(|(i, line_res)| {
            // Skip the first (header) line
            if i == 0 {
                return None;
            }
            match line_res {
                Ok(line) => Some(line),
                Err(e) => {
                    eprintln!("Error reading barcodes file at line {}: {}", i + 1, e);
                    None
                }
            }
        })
        .collect();

    // Setup a progress bar
    let pb = Arc::new(
        ProgressBar::new(barcode_lines.len() as u64).with_message("Processing barcodes...")
    );
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>3}/{len:3} {msg}")
            .unwrap()
    );

    // Process each barcode line in parallel
    barcode_lines.par_iter().for_each(|barcode_line| {
        let pb_clone = Arc::clone(&pb);
        let fields: Vec<&str> = barcode_line.trim().split('\t').collect();

        if fields.len() != 6 {
            eprintln!("Invalid line: {}", barcode_line);
            pb_clone.inc(1);
            return;
        }

        let name = fields[0];
        let file_name = fields[1];
        // The columns idx1, seq1, idx2 (fields[2], fields[3], fields[4]) are not used further.
        let seq2 = fields[5];

        // Determine the forward (R1) file (preferring gzipped, then uncompressed)
        let fq_r1_file = find_fastq(&format!("{}_R1_001.fastq", file_name));
        if fq_r1_file.is_none() {
            eprintln!("R1 file does not exist for {}", file_name);
            pb_clone.inc(1);
            return;
        }

        // Determine the reverse (R2) file
        let fq_r2_file = find_fastq(&format!("{}_R2_001.fastq", file_name));
        if fq_r2_file.is_none() {
            eprintln!("R2 file does not exist for {}", file_name);
            pb_clone.inc(1);
            return;
        }

        // Create output base (and sample ID) as "name_seq2"
        let outbase = format!("{}_{}", name, seq2);

        // Demultiplex
        if let Err(e) = demultiplex_fastq_files(
            &fq_r1_file.unwrap(),
            &fq_r2_file.unwrap(),
            seq2,
            &outbase,
        ) {
            eprintln!("Error processing {}: {}", file_name, e);
        }

        pb_clone.inc(1);
    });

    pb.finish_with_message("Done processing barcodes");
    Ok(())
}

/// Generates a QIIME2 manifest file from the barcodes file.
/// The manifest file is written to `qiime_manifest` inside [`OUTPUT_DIR`].
///
/// The expected barcodes file has the same six-column format as in
/// [`run_demultiplex_combined`].
///
/// # Errors
///
/// Returns an `io::Error` if reading the barcodes file or writing the manifest fails.
pub fn generate_qiime_manifest(barcodes_file: &str, qiime_manifest: &str) -> io::Result<()> {
    let infile = File::open(barcodes_file)?;
    let reader = BufReader::new(infile);
    let manifest_path = out_path(qiime_manifest);
    let mut writer = File::create(manifest_path)?;

    // Write the QIIME2 manifest header
    writeln!(
        writer,
        "sample-id\tforward-absolute-filepath\treverse-absolute-filepath"
    )?;

    for (i, line_res) in reader.lines().enumerate() {
        let line = line_res?;
        // Skip the header line in the barcodes file
        if i == 0 {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 6 {
            eprintln!("Skipping invalid line in barcodes file: {}", line);
            continue;
        }

        let name = fields[0];
        let seq2 = fields[5];
        let sample_id = format!("{}_{}", name, seq2);

        // Our demultiplexed FASTQ files are placed in OUTPUT_DIR and compressed with .gz
        let forward_rel = format!("{}_L001_R1_001.fastq.gz", sample_id);
        let reverse_rel = format!("{}_L001_R2_001.fastq.gz", sample_id);

        let forward_abs = fs::canonicalize(out_path(&forward_rel))?;
        let reverse_abs = fs::canonicalize(out_path(&reverse_rel))?;

        writeln!(
            writer,
            "{}\t{}\t{}",
            sample_id,
            forward_abs.display(),
            reverse_abs.display()
        )?;
    }

    Ok(())
}

/// Helper to locate FASTQ files with an optional `.gz` extension.
///
/// The function **first searches** for `base_name + ".gz"`, and if that file
/// does not exist, it then checks for `base_name` uncompressed. If neither
/// file is found, returns `None`.
fn find_fastq(base_name: &str) -> Option<String> {
    let gz = format!("{}.gz", base_name);
    if Path::new(&gz).is_file() {
        Some(gz)
    } else if Path::new(base_name).is_file() {
        Some(base_name.to_string())
    } else {
        None
    }
}

/// Reads two FASTQ files (R1, R2) and trims the adapter sequence from R1
/// (when present after the first 4 bases), then writes the resulting
/// demultiplexed FASTQ records as gzip-compressed to disk.
///
/// The new R1/R2 files are named `"{outbase}_L001_R1_001.fastq.gz"` and
/// `"{outbase}_L001_R2_001.fastq.gz"`, placed in [`OUTPUT_DIR`].
///
/// # Errors
///
/// Returns an `io::Error` if:
/// - The input files do not exist.
/// - Any file fails to open.
/// - Writing to output files fails.
fn demultiplex_fastq_files(
    fq_r1_file: &str,
    fq_r2_file: &str,
    adaptseq: &str,
    outbase: &str,
) -> io::Result<()> {
    // Verify both files exist
    if !Path::new(fq_r1_file).exists() || !Path::new(fq_r2_file).exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("File(s) do not exist: {}, {}", fq_r1_file, fq_r2_file),
        ));
    }

    // Compute final (gzipped) output file names
    let outfile1 = out_path(&format!("{}_L001_R1_001.fastq.gz", outbase));
    let outfile2 = out_path(&format!("{}_L001_R2_001.fastq.gz", outbase));

    // Open input FASTQ readers
    let in1 = open_fastq_reader(fq_r1_file)
        .map_err(|e| {
            eprintln!("Error opening file {}: {}", fq_r1_file, e);
            e
        })?;
    let in2 = open_fastq_reader(fq_r2_file)
        .map_err(|e| {
            eprintln!("Error opening file {}: {}", fq_r2_file, e);
            e
        })?;

    // Prepare gzip-compressed output writers
    let gz1 = GzEncoder::new(File::create(&outfile1)?, Compression::best());
    let gz2 = GzEncoder::new(File::create(&outfile2)?, Compression::best());

    // Wrap them as FASTQ Writers
    let mut out1 = fastq::Writer::new(gz1);
    let mut out2 = fastq::Writer::new(gz2);

    let mut records1 = in1.records();
    let mut records2 = in2.records();

    let adaptseq_bytes = adaptseq.as_bytes();
    let index_len = adaptseq_bytes.len();
    let start_idx = 4;
    let end_idx = start_idx + index_len;

    // Read pairs in lockstep
    while let Some(rec1_result) = records1.next() {
        let rec1 = rec1_result.map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let rec2 = match records2.next() {
            Some(Ok(r)) => r,
            Some(Err(e)) => return Err(io::Error::new(io::ErrorKind::Other, e)),
            None => break, // no matching second read
        };

        // If R1 has enough length and the adapter is found after the first 4 bases, trim it
        let seq1 = rec1.seq();
        let qual1 = rec1.qual();
        if seq1.len() >= end_idx && &seq1[start_idx..end_idx] == adaptseq_bytes {
            let new_seq1 = &seq1[end_idx..];
            let new_qual1 = &qual1[end_idx..];
            let new_rec1 = fastq::Record::with_attrs(rec1.id(), rec1.desc(), new_seq1, new_qual1);

            out1.write_record(&new_rec1)?;
            out2.write_record(&rec2)?;
        }
        // If the adapter isn't found, we skip writing that pair.
        // (If you want untrimmed reads to still appear, you'd add logic here.)
    }

    // Finalize the gzip writers
    out1.flush()?;
    out2.flush()?;
    Ok(())
}

/// Opens a file (gzipped or not) and returns a boxed trait object implementing `BufRead + Send`.
fn open_bufread(filename: &str) -> io::Result<Box<dyn BufRead + Send>> {
    if filename.ends_with(".gz") {
        let file = File::open(filename)?;
        let decoder = MultiGzDecoder::new(file);
        Ok(Box::new(BufReader::new(decoder)))
    } else {
        let file = File::open(filename)?;
        Ok(Box::new(BufReader::new(file)))
    }
}

/// Wraps a boxed `BufRead` in a FASTQ reader without adding another `BufReader` layer.
fn open_fastq_reader(filename: &str) -> io::Result<fastq::Reader<Box<dyn BufRead + Send>>> {
    open_bufread(filename).map(fastq::Reader::from_bufread)
}
