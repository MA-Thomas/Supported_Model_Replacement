use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Result, io};

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|error| io(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| io(path, error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex_digest(hasher.finalize()))
}

pub fn hash_serializable<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|source| crate::Error::Json {
        path: "<canonical-memory-value>".into(),
        source,
    })?;
    Ok(sha256_bytes(&bytes))
}

pub fn seed_from_hash_material<T: Serialize>(value: &T) -> Result<u64> {
    let bytes = serde_json::to_vec(value).map_err(|source| crate::Error::Json {
        path: "<seed-material>".into(),
        source,
    })?;
    let digest = Sha256::digest(bytes);
    let mut first = [0_u8; 8];
    first.copy_from_slice(&digest[..8]);
    Ok(u64::from_be_bytes(first))
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    let mut output = String::with_capacity(bytes.as_ref().len() * 2);
    for byte in bytes.as_ref() {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

pub fn canonical_vector_hash(endpoint_ids: &[String], scores: &[f64]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"directed-round-robin-score-vector-v1\0");
    for (endpoint, score) in endpoint_ids.iter().zip(scores) {
        hasher.update((endpoint.len() as u64).to_be_bytes());
        hasher.update(endpoint.as_bytes());
        hasher.update(score.to_bits().to_be_bytes());
    }
    hex_digest(hasher.finalize())
}

pub fn canonical_label_hash(endpoint_ids: &[String], labels: &[bool]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"directed-round-robin-label-vector-v1\0");
    for (endpoint, label) in endpoint_ids.iter().zip(labels) {
        hasher.update((endpoint.len() as u64).to_be_bytes());
        hasher.update(endpoint.as_bytes());
        hasher.update([u8::from(*label)]);
    }
    hex_digest(hasher.finalize())
}
