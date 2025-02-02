use std::io::{self, BufRead, BufReader, Write};
use std::fs::{self, File};
use std::path::Path;
use std::sync::Arc;
use bio::io::fastq;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

// Use the same OUTPUT_DIR constant and out_path helper.
const OUTPUT_DIR: &str = "windchime_out";
fn out_path(relative: &str) -> String {
    format!("{}/{}", OUTPUT_DIR, relative)
}

/// Runs the demultiplexing logic using the provided barcodes file.
/// (It is assumed that the barcodes file is in a six‑column format.)
pub fn run_demultiplex_combined(barcodes_file: &str) -> io::Result<()> {
    // Open the barcodes file.
    let file = File::open(barcodes_file).map_err(|e| {
        eprintln!("Unable to open barcodes file '{}': {}", barcodes_file, e);
        e
    })?;
    let reader = BufReader::new(file);

    // Read all lines from the barcodes file, skipping the header.
    let mut lines = Vec::new();
    for (i, line_result) in reader.lines().enumerate() {
        if i == 0 {
            // Skip header
            continue;
        }
        let line = line_result.map_err(|e| {
            eprintln!("Error reading barcodes file at line {}: {}", i + 1, e);
            e
        })?;
        lines.push(line);
    }

    // Setup a progress bar.
    let pb = Arc::new(
        ProgressBar::new(lines.len() as u64).with_message("Processing barcodes...")
    );
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>3}/{len:3} {msg}")
    );

    // Process each line in parallel.
    lines.par_iter().for_each(|line| {
        let pb = Arc::clone(&pb);
        let line = line.trim();
        let fields: Vec<&str> = line.split('\t').collect();

        if fields.len() != 6 {
            eprintln!("Invalid line: {}", line);
            pb.inc(1);
            return;
        }

        let name = fields[0];
        let file_name = fields[1];
        // The columns idx1, seq1, idx2 are not used further here.
        let _idx1 = fields[2];
        let _seq1 = fields[3];
        let _idx2 = fields[4];
        let seq2 = fields[5];

        // Determine the forward (R1) file.
        let fq_r1_file = if Path::new(&format!("{}_R1_001.fastq.gz", file_name)).is_file() {
            format!("{}_R1_001.fastq.gz", file_name)
        } else if Path::new(&format!("{}_R1_001.fastq", file_name)).is_file() {
            format!("{}_R1_001.fastq", file_name)
        } else {
            eprintln!("R1 file does not exist for {}", file_name);
            pb.inc(1);
            return;
        };

        // Determine the reverse (R2) file.
        let fq_r2_file = if Path::new(&format!("{}_R2_001.fastq.gz", file_name)).is_file() {
            format!("{}_R2_001.fastq.gz", file_name)
        } else if Path::new(&format!("{}_R2_001.fastq", file_name)).is_file() {
            format!("{}_R2_001.fastq", file_name)
        } else {
            eprintln!("R2 file does not exist for {}", file_name);
            pb.inc(1);
            return;
        };

        // The output base (and sample ID) is built as "name_seq2"
        let outbase = format!("{}_{}", name, seq2);

        // Prepend OUTPUT_DIR to output file names.
        let _outfile1 = format!("{}/{}_L001_R1_001.fastq.gz", OUTPUT_DIR, outbase);
        let _outfile2 = format!("{}/{}_L001_R2_001.fastq.gz", OUTPUT_DIR, outbase);

        // Perform demultiplexing.
        if let Err(e) = demultiplex_fastq_files(&fq_r1_file, &fq_r2_file, seq2, &outbase) {
            eprintln!("Error processing {}: {}", file_name, e);
        }
        pb.inc(1);
    });

    pb.finish_with_message("Done processing barcodes");
    Ok(())
}

/// Generates a QIIME2 manifest file from the barcodes file.
/// The sample IDs and output fastq file paths will be placed in OUTPUT_DIR.
pub fn generate_qiime_manifest(barcodes_file: &str, qiime_manifest: &str) -> io::Result<()> {
    let infile = File::open(barcodes_file)?;
    let reader = BufReader::new(infile);
    let manifest_path = format!("{}/{}", OUTPUT_DIR, qiime_manifest);
    let mut writer = File::create(manifest_path)?;
    // Write the QIIME2 manifest header.
    writeln!(writer, "sample-id\tforward-absolute-filepath\treverse-absolute-filepath")?;

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if i == 0 {
            // Skip header line.
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 6 {
            eprintln!("Skipping invalid line in barcodes file: {}", line);
            continue;
        }
        let name = fields[0];
        let _file_name = fields[1];
        let seq2 = fields[5];
        // Construct sample-id as "name_seq2"
        let sample_id = format!("{}_{}", name, seq2);
        // Our demultiplexed FASTQ files are placed in OUTPUT_DIR.
        let forward = format!("{}/{}_L001_R1_001.fastq.gz", OUTPUT_DIR, sample_id);
        let reverse = format!("{}/{}_L001_R2_001.fastq.gz", OUTPUT_DIR, sample_id);
        let forward_abs = fs::canonicalize(&forward)?;
        let reverse_abs = fs::canonicalize(&reverse)?;
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

/// Core demultiplexing routine.
fn demultiplex_fastq_files(
    fq_r1_file: &str,
    fq_r2_file: &str,
    adaptseq: &str,
    outbase: &str,
) -> io::Result<()> {
    let adaptseq_bytes = adaptseq.as_bytes();
    let index_len = adaptseq_bytes.len();

    if !Path::new(fq_r1_file).exists() || !Path::new(fq_r2_file).exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("File(s) do not exist: {}, {}", fq_r1_file, fq_r2_file),
        ));
    }

    let outfile1 = format!("{}/{}_L001_R1_001.fastq.gz", OUTPUT_DIR, outbase);
    let outfile2 = format!("{}/{}_L001_R2_001.fastq.gz", OUTPUT_DIR, outbase);

    // Removed non-error, non-progress output.

    // Open input FASTQ readers.
    let in1 = match open_fastq_reader(fq_r1_file) {
        Ok(reader) => reader,
        Err(e) => {
            eprintln!("Error opening file {}: {}", fq_r1_file, e);
            return Err(e);
        }
    };
    let in2 = match open_fastq_reader(fq_r2_file) {
        Ok(reader) => reader,
        Err(e) => {
            eprintln!("Error opening file {}: {}", fq_r2_file, e);
            return Err(e);
        }
    };

    // Create output FASTQ files.
    let out1_file = match File::create(&outfile1) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error creating output file {}: {}", outfile1, e);
            return Err(e);
        }
    };
    let out2_file = match File::create(&outfile2) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error creating output file {}: {}", outfile2, e);
            return Err(e);
        }
    };

    let encoder1 = GzEncoder::new(out1_file, Compression::default());
    let encoder2 = GzEncoder::new(out2_file, Compression::default());
    let mut out1 = fastq::Writer::new(encoder1);
    let mut out2 = fastq::Writer::new(encoder2);

    let mut records1 = in1.records();
    let mut records2 = in2.records();

    let mut count_good = 0;
    let mut count_total = 0;

    loop {
        let rec1 = match records1.next() {
            Some(Ok(r)) => r,
            Some(Err(e)) => return Err(io::Error::new(io::ErrorKind::Other, e)),
            None => break,
        };
        let rec2 = match records2.next() {
            Some(Ok(r)) => r,
            Some(Err(e)) => return Err(io::Error::new(io::ErrorKind::Other, e)),
            None => break,
        };

        count_total += 1;
        let seq1 = rec1.seq();
        let qual1 = rec1.qual();
        let start_idx = 4;
        let end_idx = start_idx + index_len;

        // If the R1 read contains the adapter sequence (after the first 4 bases), trim it.
        if seq1.len() >= end_idx && &seq1[start_idx..end_idx] == adaptseq_bytes {
            count_good += 1;
            let new_seq1 = &seq1[end_idx..];
            let new_qual1 = &qual1[end_idx..];
            let new_rec1 = fastq::Record::with_attrs(
                rec1.id(),
                rec1.desc(),
                new_seq1,
                new_qual1,
            );
            out1.write_record(&new_rec1)?;
            out2.write_record(&rec2)?;
        }
    }

    // Removed summary output.
    Ok(())
}

/// Opens a file (gzipped or not) and returns a boxed trait object implementing BufRead + Send.
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

/// Wrap a boxed BufRead in a FASTQ reader without adding another BufReader layer.
fn open_fastq_reader(filename: &str) -> io::Result<fastq::Reader<Box<dyn BufRead + Send>>> {
    open_bufread(filename).map(fastq::Reader::from_bufread)
}
