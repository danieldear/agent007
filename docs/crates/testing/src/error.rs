#[derive(thiserror::Error, Debug)]
pub enum TestingError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cargo nextest not installed — run: cargo install cargo-nextest")]
    MissingTool,
    #[error("nextest output parse error: {0}")]
    ParseError(String),
    #[error("pipeline stage '{stage}' failed: {reason}")]
    StageFailed { stage: String, reason: String },
    #[error("model error: {0}")]
    ModelError(String),
}

#[cfg(test)]
mod tests {
    #[test]
    fn error_variants_exist() {
        use crate::TestingError;
        let _e: TestingError = TestingError::MissingTool;
    }
}
