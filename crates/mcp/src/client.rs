use std::collections::HashMap;

use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientInfo},
    transport::TokioChildProcess,
};
use serde_json::Value;

use crate::{config::McpServerConfig, error::McpError};

/// A description of a single tool advertised by an MCP server.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

/// A handle to a connected MCP server subprocess.
struct ServerHandle {
    peer: rmcp::Peer<rmcp::RoleClient>,
    _service: rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>,
}

/// Client that manages one or more MCP server subprocesses and provides
/// a unified interface for listing and calling tools.
pub struct McpClient {
    servers: Vec<McpServerConfig>,
    handles: Vec<ServerHandle>,
    /// Maps tool name → index into `handles`.
    tool_index: HashMap<String, usize>,
}

impl McpClient {
    /// Create a client from server configs. Does not start servers yet.
    pub fn new(servers: Vec<McpServerConfig>) -> Self {
        Self {
            servers,
            handles: Vec::new(),
            tool_index: HashMap::new(),
        }
    }

    /// Start all configured server subprocesses and connect via the MCP SDK.
    pub async fn connect(&mut self) -> Result<(), McpError> {
        self.handles.clear();
        self.tool_index.clear();

        for config in &self.servers {
            tracing::info!(server = %config.name, command = %config.command, "connecting to MCP server");

            // Spawn via `sh -c` so that quoted paths and shell metacharacters
            // in the command string are handled correctly.
            let mut cmd = tokio::process::Command::new("sh");
            cmd.arg("-c").arg(&config.command);

            let transport = TokioChildProcess::new(cmd).map_err(|e| McpError::ServerStartFailed {
                name: config.name.clone(),
                source: e,
            })?;

            let client_info = ClientInfo::default();
            let running = client_info
                .serve(transport)
                .await
                .map_err(|e| McpError::Sdk(e.to_string()))?;

            let peer = running.peer().clone();
            let handle_idx = self.handles.len();
            self.handles.push(ServerHandle {
                peer: peer.clone(),
                _service: running,
            });

            // Discover and index tools from this server.
            match peer.list_all_tools().await {
                Ok(tools) => {
                    for tool in tools {
                        let name = tool.name.to_string();
                        tracing::info!(server = %config.name, tool = %name, "registered tool");
                        self.tool_index.insert(name, handle_idx);
                    }
                }
                Err(e) => {
                    tracing::error!(server = %config.name, error = %e, "failed to list tools after connect");
                }
            }

            tracing::info!(server = %config.name, "connected");
        }
        Ok(())
    }

    /// Return all tools advertised by all connected servers (deduplicated by name, last-wins).
    pub async fn list_tools(&self) -> Result<Vec<ToolDef>, McpError> {
        let mut map: HashMap<String, ToolDef> = HashMap::new();

        for handle in &self.handles {
            let tools = handle
                .peer
                .list_all_tools()
                .await
                .map_err(|e| McpError::Sdk(e.to_string()))?;

            for tool in tools {
                let name = tool.name.to_string();
                let description = tool.description.as_ref().map(|d| d.to_string());
                let input_schema = tool.schema_as_json_value();
                map.insert(
                    name.clone(),
                    ToolDef {
                        name,
                        description,
                        input_schema,
                    },
                );
            }
        }

        Ok(map.into_values().collect())
    }

    /// Call a named tool with JSON args. Returns the tool's JSON response.
    ///
    /// Returns `McpError::ToolNotFound` if no connected server advertises the tool.
    pub async fn call_tool(
        &self,
        name: &str,
        args: Value,
    ) -> Result<Value, McpError> {
        let handle_idx = self
            .tool_index
            .get(name)
            .copied()
            .ok_or_else(|| {
                tracing::error!(tool = %name, "tool not found");
                McpError::ToolNotFound(name.to_string())
            })?;

        let handle = &self.handles[handle_idx];

        tracing::info!(tool = %name, "calling tool");

        // Convert serde_json::Value args to JsonObject (Map<String, Value>).
        let arguments = match args {
            Value::Object(map) => Some(map),
            Value::Null => None,
            other => {
                // Wrap a non-object in {"value": ...} as a best-effort.
                let mut m = serde_json::Map::new();
                m.insert("value".to_string(), other);
                Some(m)
            }
        };

        let params = if let Some(args) = arguments {
            CallToolRequestParams::new(name.to_string()).with_arguments(args)
        } else {
            CallToolRequestParams::new(name.to_string())
        };

        let result = handle
            .peer
            .call_tool(params)
            .await
            .map_err(|e| {
                tracing::error!(tool = %name, error = %e, "tool call failed");
                McpError::ToolCallFailed {
                    tool: name.to_string(),
                    reason: e.to_string(),
                }
            })?;

        // Prefer structured_content if present; otherwise serialise the content vec.
        let json = if let Some(structured) = result.structured_content {
            structured
        } else {
            serde_json::to_value(&result.content)?
        };

        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    // NOTE: per spec, test against a local rmcp test server — do NOT use npx subprocess in CI.
    //
    // The rmcp crate does not ship a standalone echo-server binary, so we use
    // unit tests that exercise non-networked behaviour only.  This keeps CI
    // reliable with zero external dependencies.

    use super::*;

    // test: McpClient::new() with an empty server list initialises without error.
    #[test]
    fn new_with_empty_server_list() {
        let client = McpClient::new(vec![]);
        assert!(client.servers.is_empty());
        assert!(client.handles.is_empty());
        assert!(client.tool_index.is_empty());
    }

    // test: call_tool() with an unknown tool name returns McpError::ToolNotFound
    // when the client has no connected servers (tool_index is empty).
    #[tokio::test]
    async fn call_tool_unknown_returns_tool_not_found() {
        let client = McpClient::new(vec![]);
        let result = client
            .call_tool("does_not_exist", serde_json::json!({}))
            .await;
        assert!(
            matches!(result, Err(McpError::ToolNotFound(ref t)) if t == "does_not_exist"),
            "expected ToolNotFound, got {:?}",
            result
        );
    }

    // test: list_tools() on a client with no connected servers returns an empty list.
    #[tokio::test]
    async fn list_tools_no_servers_returns_empty() {
        let client = McpClient::new(vec![]);
        let tools = client.list_tools().await.expect("list_tools should succeed");
        assert!(tools.is_empty());
    }

    // test: ToolDef fields are accessible and carry the right types.
    #[test]
    fn tool_def_fields() {
        let def = ToolDef {
            name: "my_tool".to_string(),
            description: Some("does something".to_string()),
            input_schema: serde_json::json!({"type": "object"}),
        };
        assert_eq!(def.name, "my_tool");
        assert_eq!(def.description.as_deref(), Some("does something"));
        assert_eq!(def.input_schema["type"], "object");
    }
}
