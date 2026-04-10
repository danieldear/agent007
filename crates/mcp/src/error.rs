use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("failed to start MCP server '{name}': {source}")]
    ServerStartFailed {
        name: String,
        source: std::io::Error,
    },

    #[error("MCP SDK error: {0}")]
    Sdk(String),

    #[error("tool not found: {0}")]
    ToolNotFound(String),

    #[error("tool call failed for '{tool}': {reason}")]
    ToolCallFailed { tool: String, reason: String },

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
