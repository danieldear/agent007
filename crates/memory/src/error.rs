use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("VectorDB error: {0}")]
    VectorDb(String),

    #[error("Embedding error: {0}")]
    Embedding(String),
}
