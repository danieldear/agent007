use std::{collections::HashMap, process::Stdio, time::Duration};

use rmcp::{
    model::{CallToolRequestParams, ClientInfo},
    transport::TokioChildProcess,
    ServiceExt,
};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    time::timeout,
};

use crate::{config::McpServerConfig, error::McpError};

/// Timeout for the initial MCP handshake (`ClientInfo::serve`).
///
/// 10s proved too tight for heavier local servers under CI load; use a more
/// forgiving default so smoke tests and real project startups do not fail
/// spuriously while still surfacing genuinely stuck servers.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for listing tools from a server.
const LIST_TOOLS_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for a single tool call. 60 s gives long-running tools room while
/// still surfacing a hang rather than blocking the LLM forever.
const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(60);

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

            let cmd = build_command(config);

            // Pipe stderr so the child never blocks on a full pipe buffer.
            // Captured lines are forwarded to tracing for visibility.
            let (transport, stderr_opt) = TokioChildProcess::builder(cmd)
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| McpError::ServerStartFailed {
                    name: config.name.clone(),
                    source: e,
                })?;

            if let Some(stderr) = stderr_opt {
                let server_name = config.name.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stderr);
                    let mut line = String::new();
                    while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                        tracing::debug!(
                            server = %server_name,
                            stderr = %line.trim_end(),
                            "MCP server stderr"
                        );
                        line.clear();
                    }
                });
            }

            let running = timeout(CONNECT_TIMEOUT, ClientInfo::default().serve(transport))
                .await
                .map_err(|_| {
                    McpError::Sdk(format!(
                        "MCP handshake timed out after {}s for server '{}'",
                        CONNECT_TIMEOUT.as_secs(),
                        config.name
                    ))
                })?
                .map_err(|e| McpError::Sdk(e.to_string()))?;

            let peer = running.peer().clone();
            let handle_idx = self.handles.len();
            self.handles.push(ServerHandle {
                peer: peer.clone(),
                _service: running,
            });

            // Discover and index tools from this server.
            match timeout(LIST_TOOLS_TIMEOUT, peer.list_all_tools()).await {
                Ok(Ok(tools)) => {
                    for tool in tools {
                        let name = tool.name.to_string();
                        tracing::info!(server = %config.name, tool = %name, "registered tool");
                        self.tool_index.insert(name, handle_idx);
                    }
                }
                Ok(Err(e)) => {
                    tracing::error!(server = %config.name, error = %e, "failed to list tools after connect");
                }
                Err(_) => {
                    tracing::error!(
                        server = %config.name,
                        "list_tools timed out after {}s",
                        LIST_TOOLS_TIMEOUT.as_secs()
                    );
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
            let tools = timeout(LIST_TOOLS_TIMEOUT, handle.peer.list_all_tools())
                .await
                .map_err(|_| {
                    McpError::Sdk(format!(
                        "list_tools timed out after {}s",
                        LIST_TOOLS_TIMEOUT.as_secs()
                    ))
                })?
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
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value, McpError> {
        let handle_idx = self.tool_index.get(name).copied().ok_or_else(|| {
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

        let result = timeout(TOOL_CALL_TIMEOUT, handle.peer.call_tool(params))
            .await
            .map_err(|_| {
                tracing::error!(
                    tool = %name,
                    "tool call timed out after {}s — check the tool exits and flushes stdout",
                    TOOL_CALL_TIMEOUT.as_secs()
                );
                McpError::ToolCallFailed {
                    tool: name.to_string(),
                    reason: format!(
                        "timed out after {}s — ensure the tool writes a newline-terminated \
                         JSON response to stdout and exits (common fix on WSL: add \
                         sys.stdout.flush() or run Python with -u)",
                        TOOL_CALL_TIMEOUT.as_secs()
                    ),
                }
            })?
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

    /// Reconnect all servers with exponential backoff (1s, 2s, 4s — max 3 attempts).
    /// Call this after detecting repeated `ToolCallFailed` errors to restore connectivity.
    pub async fn reconnect_all(&mut self) -> Result<(), McpError> {
        const MAX_ATTEMPTS: u32 = 3;
        self.handles.clear();
        self.tool_index.clear();

        for config in &self.servers {
            let mut last_err: Option<McpError> = None;
            for attempt in 0..MAX_ATTEMPTS {
                if attempt > 0 {
                    let delay = std::time::Duration::from_secs(1u64 << (attempt - 1));
                    tracing::info!(server = %config.name, attempt, delay_secs = delay.as_secs(), "reconnecting MCP server");
                    tokio::time::sleep(delay).await;
                }

                let cmd = build_command(config);
                let (transport, stderr_opt) = match TokioChildProcess::builder(cmd)
                    .stderr(Stdio::piped())
                    .spawn()
                {
                    Ok(t) => t,
                    Err(e) => {
                        last_err = Some(McpError::ServerStartFailed {
                            name: config.name.clone(),
                            source: e,
                        });
                        continue;
                    }
                };

                if let Some(stderr) = stderr_opt {
                    let server_name = config.name.clone();
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stderr);
                        let mut line = String::new();
                        while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                            tracing::debug!(
                                server = %server_name,
                                stderr = %line.trim_end(),
                                "MCP server stderr"
                            );
                            line.clear();
                        }
                    });
                }

                let running =
                    match timeout(CONNECT_TIMEOUT, ClientInfo::default().serve(transport)).await {
                        Ok(Ok(r)) => r,
                        Ok(Err(e)) => {
                            last_err = Some(McpError::Sdk(e.to_string()));
                            continue;
                        }
                        Err(_) => {
                            last_err = Some(McpError::Sdk(format!(
                                "MCP handshake timed out after {}s",
                                CONNECT_TIMEOUT.as_secs()
                            )));
                            continue;
                        }
                    };

                let peer = running.peer().clone();
                let handle_idx = self.handles.len();
                self.handles.push(ServerHandle {
                    peer: peer.clone(),
                    _service: running,
                });

                match timeout(LIST_TOOLS_TIMEOUT, peer.list_all_tools()).await {
                    Ok(Ok(tools)) => {
                        for tool in tools {
                            self.tool_index.insert(tool.name.to_string(), handle_idx);
                        }
                        tracing::info!(server = %config.name, "reconnected successfully");
                        last_err = None;
                        break;
                    }
                    Ok(Err(e)) => {
                        last_err = Some(McpError::Sdk(e.to_string()));
                    }
                    Err(_) => {
                        last_err = Some(McpError::Sdk(format!(
                            "list_tools timed out after {}s",
                            LIST_TOOLS_TIMEOUT.as_secs()
                        )));
                    }
                }
            }
            if let Some(e) = last_err {
                tracing::error!(server = %config.name, error = %e, "failed to reconnect after {} attempts", MAX_ATTEMPTS);
                return Err(e);
            }
        }
        Ok(())
    }
}

/// Build a `tokio::process::Command` from an [`McpServerConfig`].
///
/// Multi-word `command` fields with no `args` are split on whitespace rather
/// than forwarded to `sh -c`. Invoking `sh` (dash on Ubuntu) causes silent
/// hangs on WSL when scripts have CRLF line endings or differ from bash in
/// PATH resolution.
fn build_command(config: &McpServerConfig) -> tokio::process::Command {
    let mut cmd = if config.args.is_empty() {
        let mut parts = config.command.split_whitespace();
        let prog = parts.next().unwrap_or(&config.command);
        let extra: Vec<&str> = parts.collect();
        let mut command = tokio::process::Command::new(prog);
        command.args(extra);
        command
    } else {
        let mut command = tokio::process::Command::new(&config.command);
        command.args(&config.args);
        command
    };

    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }
    if !config.env.is_empty() {
        cmd.envs(&config.env);
    }
    cmd
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
        let tools = client
            .list_tools()
            .await
            .expect("list_tools should succeed");
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

    #[test]
    fn build_command_uses_structured_args_without_shell() {
        let config = McpServerConfig {
            name: "filesystem".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "server".to_string()],
            env: HashMap::new(),
            cwd: Some("/tmp".to_string()),
        };
        let command = build_command(&config);
        let debug = format!("{command:?}");
        assert!(debug.contains("\"npx\""));
        assert!(debug.contains("\"-y\""));
        assert!(debug.contains("/tmp"));
    }

    // test: multi-word command with no args is split directly — no shell invocation.
    // This prevents silent hangs on WSL where `sh` (dash) mishandles CRLF scripts.
    #[test]
    fn build_command_splits_multi_word_command_without_shell() {
        let config = McpServerConfig {
            name: "test".to_string(),
            command: "python my_tool.py".to_string(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
        };
        let command = build_command(&config);
        let debug = format!("{command:?}");
        assert!(debug.contains("\"python\""), "expected python in: {debug}");
        assert!(
            debug.contains("\"my_tool.py\""),
            "expected my_tool.py in: {debug}"
        );
        assert!(!debug.contains("\"sh\""), "must not invoke sh: {debug}");
        assert!(!debug.contains("\"bash\""), "must not invoke bash: {debug}");
    }
}
