//! Error type shared across the crate.
//!
//! Selection fails closed on any provenance, schema, or numeric anomaly through
//! one typed error and a `Result` alias.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    /// Fail-closed selection or provenance error.
    #[error("{0}")]
    Selection(String),

    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("csv error in {path}: {source}")]
    Csv {
        path: PathBuf,
        #[source]
        source: csv::Error,
    },

    #[error("json error in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl SelectionError {
    pub fn msg(message: impl Into<String>) -> Self {
        SelectionError::Selection(message.into())
    }
}

pub type Result<T> = std::result::Result<T, SelectionError>;
