//! SHA-256 helper for provenance (port of `sha256_file`).

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Result, SelectionError};

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(|e| SelectionError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(|e| SelectionError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
