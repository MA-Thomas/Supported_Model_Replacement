use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("{0}")]
    Contract(String),

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

impl InputError {
    pub fn contract(message: impl Into<String>) -> Self {
        Self::Contract(message.into())
    }
}

pub type Result<T> = std::result::Result<T, InputError>;
