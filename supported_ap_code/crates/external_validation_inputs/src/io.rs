use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::error::{InputError, Result};

pub type Row = BTreeMap<String, String>;

#[derive(Clone, Debug)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Row>,
}

pub fn read_csv(path: &Path) -> Result<Table> {
    let mut reader = csv::Reader::from_path(path).map_err(|source| InputError::Csv {
        path: path.to_path_buf(),
        source,
    })?;
    let headers: Vec<String> = reader
        .headers()
        .map_err(|source| InputError::Csv {
            path: path.to_path_buf(),
            source,
        })?
        .iter()
        .map(str::to_owned)
        .collect();
    if headers.is_empty() {
        return Err(InputError::contract(format!(
            "CSV has no header: {}",
            path.display()
        )));
    }
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|source| InputError::Csv {
            path: path.to_path_buf(),
            source,
        })?;
        if record.len() != headers.len() {
            return Err(InputError::contract(format!(
                "row width differs from header width in {}",
                path.display()
            )));
        }
        rows.push(
            headers
                .iter()
                .cloned()
                .zip(record.iter().map(|value| value.trim().to_owned()))
                .collect(),
        );
    }
    Ok(Table { headers, rows })
}

pub fn write_csv(path: &Path, headers: &[String], rows: &[Row]) -> Result<()> {
    let file = File::create(path).map_err(|source| InputError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(BufWriter::new(file));
    writer
        .write_record(headers)
        .map_err(|source| InputError::Csv {
            path: path.to_path_buf(),
            source,
        })?;
    for row in rows {
        writer
            .write_record(
                headers
                    .iter()
                    .map(|header| row.get(header).map_or("", String::as_str)),
            )
            .map_err(|source| InputError::Csv {
                path: path.to_path_buf(),
                source,
            })?;
    }
    writer.flush().map_err(|source| InputError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path).map_err(|source| InputError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_reader(BufReader::new(file)).map_err(|source| InputError::Json {
        path: path.to_path_buf(),
        source,
    })
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let file = File::create(path).map_err(|source| InputError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value).map_err(|source| InputError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    writer.write_all(b"\n").map_err(|source| InputError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    writer.flush().map_err(|source| InputError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

pub fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|source| InputError::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub fn write_text(path: &Path, value: &str) -> Result<()> {
    std::fs::write(path, value).map_err(|source| InputError::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub fn copy_file(source_path: &Path, destination: &Path) -> Result<()> {
    std::fs::copy(source_path, destination).map_err(|source| InputError::Io {
        path: destination.to_path_buf(),
        source,
    })?;
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| InputError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer).map_err(|source| InputError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn require_columns(table: &Table, required: &[&str], path: &Path) -> Result<()> {
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !table.headers.iter().any(|header| header == name))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(InputError::contract(format!(
            "{} is missing columns {:?}",
            path.display(),
            missing
        )))
    }
}

pub fn field<'a>(row: &'a Row, name: &str, path: &Path) -> Result<&'a str> {
    row.get(name)
        .map(String::as_str)
        .ok_or_else(|| InputError::contract(format!("{} lacks field {name}", path.display())))
}

pub fn filename(path: &Path) -> Result<PathBuf> {
    path.file_name()
        .map(PathBuf::from)
        .ok_or_else(|| InputError::contract(format!("path has no file name: {}", path.display())))
}
