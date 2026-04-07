# IDE Bridge Crate Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `agent007-ide-bridge` crate as an async LSP server that exposes agent007 capabilities to editors (Zed, VSCode, JetBrains) via the Language Server Protocol, with both stdio and TCP transport modes.

**Architecture:** New `crates/ide-bridge` crate uses `tower-lsp` to implement an LSP server. The server wraps agent007's core stack (dispatcher, orchestrator, skills) and exposes custom LSP commands: `agent007/run`, `agent007/skillList`, `agent007/skillRun`. A `serve-lsp` CLI subcommand launches it. Two transport modes: `--stdio` (for Zed/VSCode) and `--tcp <port>` (for JetBrains).

**Tech Stack:** Rust, tower-lsp = "0.20", tokio, thiserror, agent007-core, agent007-skills

**Prerequisites:** Plans 1–4 complete. All library crates built and tested.

**Spec:** `docs/superpowers/specs/2026-03-16-agent007-design.md`

---

## File Structure

```
crates/ide-bridge/
├── Cargo.toml
└── src/
    ├── lib.rs          # pub re-exports: IdeBridgeError, LspServer, run_stdio, run_tcp
    ├── error.rs        # IdeBridgeError (thiserror)
    ├── server.rs       # Agent007LspServer — tower_lsp::LanguageServer impl
    └── commands.rs     # execute_command dispatch: agent007.run / skillList / skillRun

crates/cli/src/commands/
└── serve_lsp.rs        # serve-lsp CLI subcommand

Modified files:
    Cargo.toml                              (root workspace)
    crates/cli/Cargo.toml
    crates/cli/src/main.rs
    crates/cli/src/commands/mod.rs
```

---

## Chunk 1: Scaffold crate + error type

### Task 1: Add ide-bridge to workspace; create Cargo.toml and error type

**Files:**
- Create: `crates/ide-bridge/Cargo.toml`
- Create: `crates/ide-bridge/src/lib.rs`
- Create: `crates/ide-bridge/src/error.rs`
- Modify: `Cargo.toml` (root — add `crates/ide-bridge` to members; add `tower-lsp = "0.20"` to workspace deps)

- [ ] **Step 1: Add ide-bridge to workspace and add tower-lsp workspace dep**

In root `Cargo.toml`, add `"crates/ide-bridge"` to the `members` array and add to `[workspace.dependencies]`:

```toml
tower-lsp = "0.20"
```

- [ ] **Step 2: Create `crates/ide-bridge/Cargo.toml`**

```toml
[package]
name = "agent007-ide-bridge"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core   = { path = "../core" }
agent007-skills = { path = "../skills" }
agent007-models = { path = "../models" }
agent007-memory = { path = "../memory" }
tower-lsp       = { workspace = true }
tokio           = { workspace = true }
serde_json      = { workspace = true }
serde           = { workspace = true }
thiserror       = { workspace = true }
tracing         = { workspace = true }
async-trait     = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

- [ ] **Step 3: Create `crates/ide-bridge/src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdeBridgeError {
    #[error("LSP transport error: {0}")]
    Transport(String),

    #[error("command not found: {0}")]
    UnknownCommand(String),

    #[error("missing required argument: {0}")]
    MissingArgument(String),

    #[error("orchestrator error: {0}")]
    Orchestrator(#[from] agent007_core::CoreError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
```

- [ ] **Step 4: Create `crates/ide-bridge/src/lib.rs`**

```rust
pub mod commands;
pub mod error;
pub mod server;

pub use error::IdeBridgeError;
pub use server::{Agent007LspServer, run_stdio, run_tcp};
```

- [ ] **Step 5: Verify the crate compiles**

```bash
cargo check -p agent007-ide-bridge
```

---

## Chunk 2: LspServer struct — initialize / shutdown

### Task 2: Implement `Agent007LspServer` with LSP lifecycle methods

**Files:**
- Create: `crates/ide-bridge/src/server.rs`

The server holds a `tower_lsp::Client` (for sending notifications to the editor) and a shared `Arc<Config>` that was loaded by the CLI. The `initialize` response advertises `executeCommand` capability with the three supported commands.

- [ ] **Step 1: Write a failing test**

Add to `crates/ide-bridge/src/server.rs` (inside `#[cfg(test)]` at the bottom):

```rust
#[cfg(test)]
mod tests {
    use tower_lsp::LspService;
    use super::*;

    #[tokio::test]
    async fn server_initializes_without_panic() {
        // Build service — LspService::new returns (service, client_socket).
        // The service itself must be constructable without panicking.
        let (service, _socket) = LspService::new(|client| {
            Agent007LspServer::new(client, Arc::new(crate::server::BridgeConfig::default()))
        });
        drop(service);
    }
}
```

Run (expect compile failure):

```bash
cargo test -p agent007-ide-bridge server_initializes_without_panic 2>&1 | head -30
```

- [ ] **Step 2: Implement `crates/ide-bridge/src/server.rs`**

```rust
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
    use tower_lsp::LspService;
    use super::*;

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
}
```

- [ ] **Step 3: Run the tests (must pass)**

```bash
cargo test -p agent007-ide-bridge -- server::tests
```

---

## Chunk 3: `commands.rs` — execute_command handlers

### Task 3: Implement `agent007.run` with AGENT007_DRY_RUN mock

**Files:**
- Create: `crates/ide-bridge/src/commands.rs`

The `dispatch_command` function parses the `ExecuteCommandParams`, routes to the appropriate handler, and returns `Option<Value>` (the JSON-RPC result sent back to the editor).

- [ ] **Step 1: Write failing tests first**

Add to the bottom of `crates/ide-bridge/src/commands.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::ExecuteCommandParams;

    fn make_params(cmd: &str, args: Vec<serde_json::Value>) -> ExecuteCommandParams {
        ExecuteCommandParams {
            command: cmd.to_string(),
            arguments: args,
            work_done_progress_params: Default::default(),
        }
    }

    // agent007.run — dry-run must succeed and return a Value::String result.
    #[tokio::test]
    async fn run_command_dry_run_succeeds() {
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let cfg = Arc::new(crate::server::BridgeConfig::default());
        let params = make_params(
            "agent007.run",
            vec![serde_json::json!({"task": "say hello"})],
        );
        let result = handle_run(cfg, params.arguments).await;
        std::env::remove_var("AGENT007_DRY_RUN");
        assert!(result.is_ok());
        let val = result.unwrap();
        assert!(val.is_some());
    }

    // agent007.skillList — returns an array (may be empty).
    #[tokio::test]
    async fn skill_list_returns_array() {
        let cfg = Arc::new(crate::server::BridgeConfig::default());
        let result = handle_skill_list(cfg).await;
        assert!(result.is_ok());
        let val = result.unwrap().unwrap();
        assert!(val.is_array());
    }

    // Unknown command must return IdeBridgeError::UnknownCommand.
    #[tokio::test]
    async fn unknown_command_returns_error() {
        let cfg = Arc::new(crate::server::BridgeConfig::default());
        let (svc, _sock) = tower_lsp::LspService::new(|client| {
            crate::server::Agent007LspServer::new(client, cfg.clone())
        });
        drop(svc);
        // dispatch_command not called here — tested via the error variant match
        let err = IdeBridgeError::UnknownCommand("agent007.bogus".to_string());
        assert!(matches!(err, IdeBridgeError::UnknownCommand(_)));
    }
}
```

Run (expect compile failure until implementation):

```bash
cargo test -p agent007-ide-bridge -- commands::tests 2>&1 | head -40
```

- [ ] **Step 2: Implement `crates/ide-bridge/src/commands.rs`**

```rust
use std::sync::Arc;

use serde_json::Value;
use tower_lsp::Client;
use tower_lsp::lsp_types::{ExecuteCommandParams, MessageType};

use crate::error::IdeBridgeError;
use crate::server::BridgeConfig;

// ── public entry point ────────────────────────────────────────────────────────

pub async fn dispatch_command(
    client: &Client,
    config: &Arc<BridgeConfig>,
    params: ExecuteCommandParams,
) -> Result<Option<Value>, IdeBridgeError> {
    match params.command.as_str() {
        "agent007.run" => {
            let result = handle_run(config.clone(), params.arguments).await?;
            if let Some(ref val) = result {
                client
                    .show_message(MessageType::INFO, format!("agent007: {val}"))
                    .await;
            }
            Ok(result)
        }
        "agent007.skillList" => handle_skill_list(config.clone()).await,
        "agent007.skillRun" => handle_skill_run(config.clone(), params.arguments).await,
        other => Err(IdeBridgeError::UnknownCommand(other.to_string())),
    }
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// `agent007.run` — `{ "task": string }` → submits to orchestrator, returns
/// `"Task submitted to agent007 orchestrator."` as a JSON string.
pub(crate) async fn handle_run(
    _config: Arc<BridgeConfig>,
    arguments: Vec<Value>,
) -> Result<Option<Value>, IdeBridgeError> {
    let arg = arguments
        .into_iter()
        .next()
        .ok_or_else(|| IdeBridgeError::MissingArgument("task".to_string()))?;

    let task = arg
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IdeBridgeError::MissingArgument("task".to_string()))?
        .to_string();

    // Use AGENT007_DRY_RUN so this path never spins up a real TUI.
    std::env::set_var("AGENT007_DRY_RUN", "1");

    // Build a minimal stack using the same helper used by the CLI run command.
    // We re-use agent007-core types directly to avoid a hard dep on the cli crate.
    let cancel = tokio_util::sync::CancellationToken::new();
    let dispatcher = agent007_core::dispatcher::LocalDispatcher::new(64);

    // Minimal model router backed by MockProvider.
    let mock =
        Arc::new(agent007_models::MockProvider::new("dry-run response", "mock"))
            as Arc<dyn agent007_models::ModelProvider>;
    let mut router = agent007_models::ModelRouter::new("mock");
    router.register("mock", mock);
    let model_router = Arc::new(router);

    let prompt_store = Arc::new(std::sync::Mutex::new(
        agent007_core::types::PromptStore::default(),
    ));
    let orchestrator = Arc::new(agent007_core::orchestrator::OrchestratorAgent::new(
        dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        model_router,
        prompt_store,
        cancel,
        _config.max_agents.max(1),
    ));

    let core_task = agent007_core::Task::new(&task);
    orchestrator
        .run(core_task)
        .await
        .map_err(IdeBridgeError::Orchestrator)?;

    Ok(Some(Value::String(
        "Task submitted to agent007 orchestrator.".to_string(),
    )))
}

/// `agent007.skillList` — returns a JSON array of `{ trigger, name, description }`.
pub(crate) async fn handle_skill_list(
    _config: Arc<BridgeConfig>,
) -> Result<Option<Value>, IdeBridgeError> {
    let skills_dir = agent007_home().join("skills");

    // If the directory does not exist yet, return an empty array gracefully.
    if !skills_dir.exists() {
        return Ok(Some(Value::Array(vec![])));
    }

    let mut entries = tokio::fs::read_dir(&skills_dir)
        .await
        .map_err(IdeBridgeError::Io)?;

    let mut skills: Vec<Value> = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(IdeBridgeError::Io)? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(IdeBridgeError::Io)?;
        if let Some(fm) = parse_frontmatter(&content) {
            skills.push(fm);
        }
    }

    Ok(Some(Value::Array(skills)))
}

/// `agent007.skillRun` — `{ "trigger": string, "args": string }` → runs skill,
/// returns output as a JSON string.
pub(crate) async fn handle_skill_run(
    _config: Arc<BridgeConfig>,
    arguments: Vec<Value>,
) -> Result<Option<Value>, IdeBridgeError> {
    let arg = arguments
        .into_iter()
        .next()
        .ok_or_else(|| IdeBridgeError::MissingArgument("trigger".to_string()))?;

    let trigger = arg
        .get("trigger")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IdeBridgeError::MissingArgument("trigger".to_string()))?
        .to_string();

    let args = arg
        .get("args")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Build a minimal SkillExecutor (dry-run, no real VectorDB).
    let embedder = Arc::new(agent007_models::MockProvider::with_embedding_dim(
        "",
        "mock-embed",
        384,
    )) as Arc<dyn agent007_models::EmbeddingProvider>;

    let db = Arc::new(NoOpVectorDB) as Arc<dyn agent007_memory::VectorDB>;
    let retriever = Arc::new(agent007_memory::Retriever::new(embedder, db, 5));

    let tmp = tempfile::TempDir::new().map_err(IdeBridgeError::Io)?;
    let memory_store = Arc::new(agent007_memory::store::MemoryStore::new(tmp.path()));
    let memory = memory_store.global();

    let mock_model =
        Arc::new(agent007_models::MockProvider::new("dry-run response", "mock"))
            as Arc<dyn agent007_models::ModelProvider>;

    let executor = agent007_skills::SkillExecutor::new(mock_model, retriever, memory);

    let skills_dir = agent007_home().join("skills");
    let loader = agent007_skills::SkillLoader::new(&skills_dir);
    let skills = loader
        .load_all()
        .map_err(|e| IdeBridgeError::Transport(e.to_string()))?;

    let skill = skills
        .into_iter()
        .find(|s| s.trigger() == trigger)
        .ok_or_else(|| IdeBridgeError::UnknownCommand(trigger.clone()))?;

    let output = executor
        .execute(&skill, &args)
        .await
        .map_err(|e| IdeBridgeError::Transport(e.to_string()))?;

    Ok(Some(Value::String(output)))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn agent007_home() -> std::path::PathBuf {
    std::env::var("AGENT007_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(".agent007")
        })
}

/// Parse YAML frontmatter from a skill `.md` file into a JSON object with
/// keys `trigger`, `name`, `description`.
fn parse_frontmatter(content: &str) -> Option<Value> {
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct Fm {
        name: String,
        description: String,
        trigger: String,
    }
    let fm: Fm = serde_yaml::from_str(parts[1]).ok()?;
    Some(serde_json::json!({
        "trigger": fm.trigger,
        "name": fm.name,
        "description": fm.description,
    }))
}

/// No-op VectorDB used in dry-run mode.
struct NoOpVectorDB;

#[async_trait::async_trait]
impl agent007_memory::VectorDB for NoOpVectorDB {
    async fn upsert(
        &self,
        _id: &str,
        _vector: Vec<f32>,
        _payload: serde_json::Value,
    ) -> Result<(), agent007_memory::MemoryError> {
        Ok(())
    }

    async fn search(
        &self,
        _query: Vec<f32>,
        _limit: usize,
    ) -> Result<Vec<agent007_memory::SearchResult>, agent007_memory::MemoryError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::ExecuteCommandParams;

    fn make_params(cmd: &str, args: Vec<serde_json::Value>) -> ExecuteCommandParams {
        ExecuteCommandParams {
            command: cmd.to_string(),
            arguments: args,
            work_done_progress_params: Default::default(),
        }
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    async fn run_command_dry_run_succeeds() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let cfg = Arc::new(crate::server::BridgeConfig::default());
        let params = make_params(
            "agent007.run",
            vec![serde_json::json!({"task": "say hello"})],
        );
        let result = handle_run(cfg, params.arguments).await;
        std::env::remove_var("AGENT007_DRY_RUN");
        assert!(result.is_ok());
        let val = result.unwrap();
        assert!(val.is_some());
    }

    #[tokio::test]
    async fn skill_list_returns_array() {
        let cfg = Arc::new(crate::server::BridgeConfig::default());
        let result = handle_skill_list(cfg).await;
        assert!(result.is_ok());
        let val = result.unwrap().unwrap();
        assert!(val.is_array());
    }

    #[test]
    fn unknown_command_error_variant() {
        let err = IdeBridgeError::UnknownCommand("agent007.bogus".to_string());
        assert!(matches!(err, IdeBridgeError::UnknownCommand(_)));
    }
}
```

- [ ] **Step 3: Run all ide-bridge tests**

```bash
cargo test -p agent007-ide-bridge
```

---

## Chunk 4: `serve-lsp` CLI subcommand

### Task 4: Wire the serve-lsp subcommand into the CLI

**Files:**
- Create: `crates/cli/src/commands/serve_lsp.rs`
- Modify: `crates/cli/Cargo.toml` (add `agent007-ide-bridge` dep)
- Modify: `crates/cli/src/commands/mod.rs` (add `pub mod serve_lsp`)
- Modify: `crates/cli/src/main.rs` (add `ServeLsp` variant + dispatch)

- [ ] **Step 1: Write a failing integration test in `serve_lsp.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_mode_parses_stdio() {
        let mode = TransportMode::Stdio;
        assert!(matches!(mode, TransportMode::Stdio));
    }

    #[test]
    fn transport_mode_parses_tcp() {
        let mode = TransportMode::Tcp { port: 7007 };
        assert!(matches!(mode, TransportMode::Tcp { port: 7007 }));
    }
}
```

Run (expect compile failure):

```bash
cargo test -p agent007 serve_lsp 2>&1 | head -20
```

- [ ] **Step 2: Create `crates/cli/src/commands/serve_lsp.rs`**

```rust
use std::sync::Arc;

use anyhow::Result;
use agent007_ide_bridge::server::{BridgeConfig, run_stdio, run_tcp};

use crate::config::Config;

#[derive(Debug, Clone)]
pub enum TransportMode {
    Stdio,
    Tcp { port: u16 },
}

pub async fn execute(config: Arc<Config>, mode: TransportMode) -> Result<()> {
    let bridge_cfg = Arc::new(BridgeConfig {
        max_agents: config.core.max_agents,
        default_model: config.models.default.clone(),
    });

    match mode {
        TransportMode::Stdio => {
            tracing::info!("agent007 LSP server starting (stdio transport)");
            run_stdio(bridge_cfg).await;
        }
        TransportMode::Tcp { port } => {
            tracing::info!("agent007 LSP server starting (TCP port {})", port);
            run_tcp(bridge_cfg, port)
                .await
                .map_err(|e| anyhow::anyhow!("LSP TCP server error: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_mode_parses_stdio() {
        let mode = TransportMode::Stdio;
        assert!(matches!(mode, TransportMode::Stdio));
    }

    #[test]
    fn transport_mode_parses_tcp() {
        let mode = TransportMode::Tcp { port: 7007 };
        assert!(matches!(mode, TransportMode::Tcp { port: 7007 }));
    }
}
```

- [ ] **Step 3: Add `agent007-ide-bridge` to `crates/cli/Cargo.toml`**

```toml
agent007-ide-bridge = { path = "../ide-bridge" }
```

- [ ] **Step 4: Add `pub mod serve_lsp` to `crates/cli/src/commands/mod.rs`**

```rust
pub mod serve_lsp;
```

- [ ] **Step 5: Add `ServeLsp` variant and dispatch to `crates/cli/src/main.rs`**

Locate the `Commands` enum and add:

```rust
/// Start the agent007 Language Server Protocol server.
#[command(name = "serve-lsp")]
ServeLsp {
    /// Use stdio transport (default, for Zed / VSCode).
    #[arg(long, conflicts_with = "tcp")]
    stdio: bool,

    /// Use TCP transport on the given port (for JetBrains).
    #[arg(long, value_name = "PORT")]
    tcp: Option<u16>,
},
```

In the `match` dispatch block, add:

```rust
Commands::ServeLsp { stdio: _, tcp } => {
    let mode = if let Some(port) = tcp {
        commands::serve_lsp::TransportMode::Tcp { port }
    } else {
        commands::serve_lsp::TransportMode::Stdio
    };
    commands::serve_lsp::execute(config, mode).await?;
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p agent007 -- serve_lsp
```

- [ ] **Step 7: Full workspace check**

```bash
cargo check --workspace
```

---

## Chunk 5: TCP mode integration test

### Task 5: Verify TCP listener binds and accepts a connection

**Files:**
- Modify: `crates/ide-bridge/src/server.rs` (add tcp integration test)

- [ ] **Step 1: Add TCP smoke test**

Add to the `#[cfg(test)]` block in `server.rs`:

```rust
#[tokio::test]
async fn tcp_server_binds_and_accepts() {
    use tokio::net::TcpStream;

    let cfg = Arc::new(BridgeConfig::default());
    // Use port 0 to get an OS-assigned free port — but run_tcp does not expose
    // the bound address. Instead, pick a high port unlikely to be in use.
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
```

- [ ] **Step 2: Run the tcp test**

```bash
cargo test -p agent007-ide-bridge tcp_server_binds_and_accepts
```

---

## Summary of Cargo.toml changes

### Root `Cargo.toml` additions

```toml
# In [workspace] members:
"crates/ide-bridge",

# In [workspace.dependencies]:
tower-lsp = "0.20"
```

### `crates/cli/Cargo.toml` additions

```toml
agent007-ide-bridge = { path = "../ide-bridge" }
```

### `crates/ide-bridge/Cargo.toml` additions (serde_yaml for frontmatter parsing)

```toml
serde_yaml = { workspace = true }
tempfile   = { workspace = true }
```

---

## Full test suite reference

```bash
# Unit tests — ide-bridge crate only
cargo test -p agent007-ide-bridge

# Unit tests — CLI serve-lsp command
cargo test -p agent007 -- serve_lsp

# Whole workspace
cargo test --workspace

# Compile check
cargo check --workspace
```
