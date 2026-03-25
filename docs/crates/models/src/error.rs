use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("api error from {provider}: {message}")]
    Api { provider: String, message: String },
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("not configured: {0}")]
    NotConfigured(String),
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_error_displays() {
        let e = ModelError::NotConfigured("claude".to_string());
        assert!(e.to_string().contains("claude"));
    }
}
