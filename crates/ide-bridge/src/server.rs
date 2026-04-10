use std::sync::Arc;

use serde_json::Value;
use tower_lsp::jsonrpc;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::commands::dispatch_command;
use crate::error::IdeBridgeError;

/// Minimal config the LSP server needs (subset of the CLI Config).
#[derive(Debug, Default, Clone)]
pub struct BridgeConfig {
    /// Max number of agents to spawn per task.
    pub max_agents: usize,
    /// Default model provider name.
    pub default_model: String,
}

pub struct Agent007LspServer {
    pub(crate) client: Client,
    pub(crate) config: Arc<BridgeConfig>,
}

impl Agent007LspServer {
    pub fn new(client: Client, config: Arc<BridgeConfig>) -> Self {
        Self { client, config }
    }

    /// Names of all commands this server supports.
    fn supported_commands() -> Vec<String> {
        vec![
            "agent007.run".to_string(),
            "agent007.skillList".to_string(),
            "agent007.skillRun".to_string(),
        ]
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Agent007LspServer {
    async fn initialize(&self, _params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: Self::supported_commands(),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "agent007-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "agent007 LSP server initialized")
            .await;
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> jsonrpc::Result<Option<Value>> {
        self.client
            .log_message(
                MessageType::INFO,
                format!("execute_command: {}", params.command),
            )
            .await;

        match dispatch_command(&self.client, &self.config, params).await {
            Ok(result) => Ok(result),
            Err(IdeBridgeError::UnknownCommand(cmd)) => Err(jsonrpc::Error {
                code: jsonrpc::ErrorCode::MethodNotFound,
                message: format!("unknown command: {cmd}").into(),
                data: None,
            }),
            Err(e) => Err(jsonrpc::Error {
                code: jsonrpc::ErrorCode::InternalError,
                message: e.to_string().into(),
                data: None,
            }),
        }
    }
}

/// Start the server in stdio mode (Zed / VSCode).
pub async fn run_stdio(config: Arc<BridgeConfig>) {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) =
        tower_lsp::LspService::new(|client| Agent007LspServer::new(client, config));
    tower_lsp::Server::new(stdin, stdout, socket)
        .serve(service)
        .await;
}

/// Start the server in TCP mode (JetBrains). Binds `0.0.0.0:<port>`.
pub async fn run_tcp(config: Arc<BridgeConfig>, port: u16) -> Result<(), IdeBridgeError> {
    use tokio::net::TcpListener;

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("agent007 LSP server listening on TCP {addr}");

    loop {
        let (stream, peer) = listener.accept().await?;
        tracing::info!("LSP client connected from {peer}");
        let cfg = config.clone();
        tokio::spawn(async move {
            let (read, write) = tokio::io::split(stream);
            let (service, socket) =
                tower_lsp::LspService::new(|client| Agent007LspServer::new(client, cfg));
            tower_lsp::Server::new(read, write, socket)
                .serve(service)
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::LspService;

    #[tokio::test]
    async fn server_initializes_without_panic() {
        let (service, _socket) = LspService::new(|client| {
            Agent007LspServer::new(client, Arc::new(BridgeConfig::default()))
        });
        drop(service);
    }

    #[tokio::test]
    async fn supported_commands_contains_all_three() {
        let cmds = Agent007LspServer::supported_commands();
        assert!(cmds.contains(&"agent007.run".to_string()));
        assert!(cmds.contains(&"agent007.skillList".to_string()));
        assert!(cmds.contains(&"agent007.skillRun".to_string()));
    }

    #[tokio::test]
    async fn tcp_server_binds_and_accepts() {
        use tokio::net::TcpStream;

        let cfg = Arc::new(BridgeConfig::default());
        // Use a high port unlikely to be in use.
        let port: u16 = 17007;
        let cfg_clone = cfg.clone();

        let server_handle = tokio::spawn(async move {
            // run_tcp loops forever; cancel after first connection is handled.
            tokio::time::timeout(
                std::time::Duration::from_millis(500),
                run_tcp(cfg_clone, port),
            )
            .await
            .ok();
        });

        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // A plain TCP connect should succeed (LSP handshake is not required here).
        let connect = TcpStream::connect(format!("127.0.0.1:{port}")).await;
        assert!(connect.is_ok(), "TCP connect failed: {:?}", connect.err());

        server_handle.abort();
    }
}
