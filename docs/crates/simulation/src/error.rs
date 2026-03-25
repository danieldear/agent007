use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum SimulationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse template {path}: {reason}")]
    ParseError { path: PathBuf, reason: String },
    #[error("template '{name}' not found")]
    TemplateNotFound { name: String },
    #[error("system under test failed (exit code {code}): {stderr}")]
    SutFailed { code: i32, stderr: String },
    #[error("scenario '{name}' timed out after {secs}s")]
    Timeout { name: String, secs: u64 },
    #[error("validation failed for scenario '{name}': {reason}")]
    ValidationFailed { name: String, reason: String },
    #[error("model error: {0}")]
    ModelError(String),
}
