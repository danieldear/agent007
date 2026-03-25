use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("address bind error on {addr}: {source}")]
    Bind {
        addr: String,
        source: std::io::Error,
    },

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("orchestrator error: {0}")]
    Orchestrator(#[from] agent007_core::CoreError),

    #[error("skill error: {0}")]
    Skill(String),
}
