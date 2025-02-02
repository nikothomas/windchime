// biom_converter.rs

use std::error::Error;
use std::fs::File;
use std::io::BufReader;

use csv::WriterBuilder;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Biom {
    /// BIOM “shape” field must be an array with two numbers: [num_rows, num_cols]
    shape: Vec<usize>,
    /// Sparse matrix data – each entry is expected to be a 3‐element array:
    /// [row_index, col_index, value]
    data: Vec<Vec<Value>>,
    rows: Vec<BiomRow>,
    columns: Vec<BiomColumn>,
    // Other fields are ignored.
}

#[derive(Deserialize)]
struct BiomRow {
    id: String,
    metadata: Option<Value>,
}

#[derive(Deserialize)]
struct BiomColumn {
    id: String,
    metadata: Option<Value>,
}

/// Converts a BIOM file (in JSON format) to a TSV file.
///
/// The TSV file will have a header with “Feature ID” as the first column,
/// followed by sample IDs (from the BIOM “columns”). Each subsequent row contains
/// the feature ID (from the BIOM “rows”) and the count values (filling in zeros when
/// no value is present).
pub fn convert_biom_to_tsv(input_path: &str, output_path: &str) -> Result<(), Box<dyn Error>> {
    // Open the BIOM file.
    let file = File::open(input_path)?;
    let reader = BufReader::new(file);
    let biom: Biom = serde_json::from_reader(reader)?;

    // Validate the "shape" field.
    if biom.shape.len() != 2 {
        return Err("Invalid BIOM file: 'shape' field must have two elements".into());
    }
    let n_rows = biom.shape[0];
    let n_cols = biom.shape[1];

    if n_rows != biom.rows.len() || n_cols != biom.columns.len() {
        return Err("Mismatch between 'shape' and number of rows/columns in BIOM file".into());
    }

    // Create a dense matrix (rows x columns) initialized to zero.
    let mut matrix = vec![vec![0.0; n_cols]; n_rows];

    // Populate the matrix with the nonzero values from the sparse data array.
    for entry in biom.data.iter() {
        if entry.len() != 3 {
            return Err("Invalid data entry in BIOM file; expected three elements".into());
        }
        // Extract indices and value.
        let row_index = entry[0]
            .as_u64()
            .ok_or("Invalid row index in BIOM data entry")? as usize;
        let col_index = entry[1]
            .as_u64()
            .ok_or("Invalid column index in BIOM data entry")? as usize;
        let value = entry[2]
            .as_f64()
            .ok_or("Invalid value in BIOM data entry")?;

        if row_index >= n_rows || col_index >= n_cols {
            return Err("Data entry index out of bounds".into());
        }
        matrix[row_index][col_index] = value;
    }

    // Open the output file and create a CSV writer configured for TSV output.
    let mut wtr = WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(output_path)?;

    // Write header: "Feature ID" followed by sample IDs (from columns).
    let mut header = Vec::with_capacity(n_cols + 1);
    header.push("Feature ID".to_string());
    for col in biom.columns.iter() {
        header.push(col.id.clone());
    }
    wtr.write_record(&header)?;

    // Write one row per feature.
    for (i, row) in biom.rows.iter().enumerate() {
        let mut record = Vec::with_capacity(n_cols + 1);
        record.push(row.id.clone());
        for &val in &matrix[i] {
            // Format the value: if it is an integer value, show no decimal point.
            if (val - val.trunc()).abs() < std::f64::EPSILON {
                record.push(format!("{}", val as i64));
            } else {
                record.push(format!("{}", val));
            }
        }
        wtr.write_record(&record)?;
    }
    wtr.flush()?;
    println!("Converted BIOM file '{}' to TSV file '{}'", input_path, output_path);
    Ok(())
}
