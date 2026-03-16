// TODO: Implement McpClient using rmcp SDK
// This is a stub implementation to be filled in later.

use crate::error::McpError;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct McpClient {
    // TODO: add rmcp client fields
}

impl McpClient {
    pub async fn call_tool(&self, tool: &str, _args: Value) -> Result<Value, McpError> {
        Err(McpError::ToolNotFound(tool.to_string()))
    }
}
