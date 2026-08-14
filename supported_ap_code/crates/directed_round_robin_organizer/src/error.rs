use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("JSON error at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("CSV error at {path}: {source}")]
    Csv {
        path: PathBuf,
        #[source]
        source: csv::Error,
    },
    #[error("invalid bundle: {0}")]
    InvalidBundle(String),
    #[error("invalid tournament specification: {0}")]
    InvalidSpec(String),
    #[error("invalid plan: {0}")]
    InvalidPlan(String),
    #[error("judge failed: {0}")]
    Judge(String),
    #[error("artifact conflict: {0}")]
    ArtifactConflict(String),
    #[error("tournament is operationally incomplete: {0}")]
    Incomplete(String),
    #[error("invalid reduction: {0}")]
    InvalidReduction(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Error {
    Error::Io {
        path: path.into(),
        source,
    }
}
