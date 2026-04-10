use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Task queue full — backpressure limit reached")]
    TaskQueueFull,

    #[error("Channel disconnected — receiver dropped")]
    Disconnected,

    #[error("Dispatcher publish failed: {0}")]
    DispatchFailed(String),

    #[error("Model error: {0}")]
    Model(#[from] agent007_models::ModelError),

    #[error("MCP error: {0}")]
    Mcp(#[from] agent007_mcp::McpError),

    #[error("resource not configured: {0}")]
    NotConfigured(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Shutdown in progress")]
    ShuttingDown,
}

impl CoreError {
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_displays_agent_id() {
        let e = CoreError::AgentNotFound("abc-123".to_string());
        assert!(e.to_string().contains("abc-123"));
    }
}
