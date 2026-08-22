//! CSV / gzip-CSV writers used for the report files. Rows are pre-stringified
//! so the caller controls formatting; `f64` uses Rust's shortest round-trip
//! representation.

use std::io::Write;
use std::path::Path;

use flate2::write::GzEncoder;
use flate2::{Compression, GzBuilder};

use crate::error::{Result, SelectionError};

fn io_err(path: &Path, source: std::io::Error) -> SelectionError {
    SelectionError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Format an `f64` for CSV output. Shortest round-trip representation, but
/// integer-valued floats keep a trailing `.0` (`-2.0`, `0.0`) to match pandas'
/// float rendering; `inf`/`nan` match NumPy's CSV tokens.
pub fn fmt_f64(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let s = format!("{value}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn render(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push_str(&header.join(","));
    out.push('\n');
    for row in rows {
        // fields here never contain commas or quotes (identifiers and numbers),
        // matching the pandas default for this data.
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

pub fn write_csv(path: &Path, header: &[&str], rows: &[Vec<String>]) -> Result<()> {
    let body = render(header, rows);
    std::fs::write(path, body).map_err(|e| io_err(path, e))
}

pub fn write_csv_gz(path: &Path, header: &[&str], rows: &[Vec<String>]) -> Result<()> {
    let body = render(header, rows);
    let file = std::fs::File::create(path).map_err(|e| io_err(path, e))?;
    // mtime 0 keeps the gzip container reproducible (mirrors pandas mtime=0).
    let mut encoder: GzEncoder<std::fs::File> =
        GzBuilder::new().mtime(0).write(file, Compression::new(6));
    encoder
        .write_all(body.as_bytes())
        .map_err(|e| io_err(path, e))?;
    encoder.finish().map_err(|e| io_err(path, e))?;
    Ok(())
}

/// Stream rows directly into a reproducible gzip CSV. This avoids materializing
/// the large joint-grid surface tables as strings in memory.
pub fn write_csv_gz_iter<I>(path: &Path, header: &[&str], rows: I) -> Result<()>
where
    I: IntoIterator<Item = Vec<String>>,
{
    let file = std::fs::File::create(path).map_err(|e| io_err(path, e))?;
    let encoder = GzBuilder::new().mtime(0).write(file, Compression::new(6));
    let mut writer = csv::WriterBuilder::new().from_writer(encoder);
    writer
        .write_record(header)
        .map_err(|e| SelectionError::Csv {
            path: path.to_path_buf(),
            source: e,
        })?;
    for row in rows {
        writer.write_record(&row).map_err(|e| SelectionError::Csv {
            path: path.to_path_buf(),
            source: e,
        })?;
    }
    writer.flush().map_err(|e| io_err(path, e))?;
    let encoder = writer
        .into_inner()
        .map_err(|error| io_err(path, error.into_error()))?;
    encoder.finish().map_err(|e| io_err(path, e))?;
    Ok(())
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text).map_err(|e| io_err(path, e))
}
