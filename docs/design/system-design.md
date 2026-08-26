# agent007 — System Design

*Current-state design document. Describes the code as it exists, not aspirational plans.*

---

## 1. System Overview

agent007 is a Rust AI-orchestration server that exposes its capabilities over the
[MCP (Model Context Protocol)](https://modelcontextprotocol.io/) stdio transport. An editor
plugin (Zed, VS Code, etc.) launches the binary and speaks JSON-RPC over stdin/stdout. The
server owns 44 tool definitions, routes LLM calls through a multi-provider model router,
stores persistent memory on disk and in LanceDB, executes YAML-defined multi-agent workflows,
and serves a local web dashboard on port 8007.

### 2026-05-03 Addendum

Current system surface also includes:

1. Extension adapters and extension install/list APIs.
2. MCP registry and RAG source management API routes.
3. Tool registry import/search/test/approve lifecycle.
4. Memory stats API (`/api/memory/{scope}/stats`) and dashboard observability.
5. Runtime learning workers active in both `run` and `serve` code paths.

### Design philosophy

| Principle | How it is applied |
|-----------|-------------------|
| **Library / binary split** | Every feature lives in a library crate. `cli` is the thin binary that wires them together. |
| **Thiserror in libraries, anyhow in CLI** | Each library crate declares typed errors with `thiserror`. The `cli` crate collapses them into `anyhow::Error` at the boundary. |
| **Tokio everywhere** | All async code targets the Tokio runtime. No mixed runtimes. |
| **Arc + async traits** | Shared state is wrapped in `Arc<dyn Trait>`. Trait objects are object-safe; `async_trait` is used for async methods. |
| **Config-over-code** | Skills, workflows, and personas are files on disk, not compiled-in resources. |
| **Home directory discovery** | Walks up from CWD looking for `.agent007/`, falls back to `~/.agent007/`. Mirrors git's `.git/` convention. |

---

## 2. Crate Dependency Graph

```
cli (binary)
 ├── core
 │    ├── models
 │    ├── mcp
 │    └── zones
 ├── models          (no internal deps)
 ├── memory
 │    └── models
 ├── skills
 │    ├── models
 │    └── memory
 ├── hooks           (no internal deps)
 ├── learning
 │    ├── core
 │    ├── memory
 │    └── models
 ├── personas
 │    └── core
 ├── workflows
 │    ├── core
 │    ├── models
 │    └── skills
 ├── mcp             (no internal deps; wraps rmcp)
 ├── zones           (no internal deps)
 ├── tui             (no internal deps; ratatui)
 ├── web             (Axum; no internal deps)
 ├── ide-bridge      (tower-lsp; no internal deps)
 ├── git-agent       (git2; no internal deps)
 ├── custom-agents   (no internal deps)
 ├── testing         (no internal deps)
 └── simulation      (no internal deps)
```

`core` is the hub. It defines the traits and types that `models`, `skills`, `workflows`,
`learning`, and `personas` all depend on. `cli` depends on everything.

---

## 3. Data Flow: Editor MCP Call → Response

```
Editor (Zed / VS Code)
  │  JSON-RPC over stdin/stdout (rmcp crate)
  ▼
cli/src/serve.rs  ─── McpServer (44 tool handlers)
  │
  ├─ Memory tools ──────────────────────► MemoryStore (filesystem, ~/.agent007/memory/)
  │                                            └── ScopedMemoryStore (Arc<MemoryStore>)
  │
  ├─ Skill tools ───────────────────────► SkillLoader → SkillExecutor
  │                                            └── Tera template render → CompletionRequest
  │                                                  └── ModelRouter.route(task_type)
  │                                                        └── Arc<dyn ModelProvider>.complete()
  │                                                              └── HTTP to Claude / Codex / Ollama
  │
  ├─ Workflow tools ────────────────────► WorkflowLoader → HostedWorkflowEngine
  │   (workflow_start / next / submit)        └── DAG validated by DagValidator (petgraph)
  │                                                state persisted as JSON in RunStore
  │                                                approval gates → PendingApproval
  │
  ├─ Persona tools ─────────────────────► PersonaRegistry (TOML files in ~/.agent007/personas/)
  │
  ├─ Learning tools ────────────────────► LearningStore → FeedbackEntry (written on record_tokens)
  │
  ├─ Git tools ─────────────────────────► git-agent crate (git2)
  │
  └─ MCP proxy tools ───────────────────► McpClient (downstream MCP servers in config)

                    ┌────────────────────────────────────────┐
                    │  Side-channel: LocalDispatcher         │
                    │  (tokio broadcast channel, cap=256)    │
                    │  Subscribers: TUI dashboard, web WS,   │
                    │  RunStore log writer                    │
                    └────────────────────────────────────────┘
```

The MCP handler in `serve.rs` is the single entry point. It is a large match on 44 tool
names (~4,350 lines). Each arm calls into the appropriate library crate; results are
serialized back as `serde_json::Value` and returned over stdio.

---

## 4. Key Traits and Abstractions

### 4.1 `ModelProvider` (`crates/models/src/provider.rs`)

```rust
#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError>;
}
```

Implementations: `ClaudeProvider`, `CodexProvider`, `OllamaProvider`, `MockProvider`.

`CompletionResponse` carries `input_tokens: Option<u32>` and `output_tokens: Option<u32>`;
providers that return token counts populate them; those that do not leave them `None`.

### 4.2 `EmbeddingProvider` (`crates/models/src/provider.rs`)

```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ModelError>;
}
```

Used by `LanceDBStore` for semantic search. No concrete implementation ships yet beyond
`MockProvider`; the embedding model is a known gap (see §9).

### 4.3 `Dispatcher` (`crates/core/src/dispatcher.rs`)

```rust
#[async_trait]
pub trait Dispatcher: Send + Sync {
    async fn publish(&self, event: AgentEvent) -> Result<(), CoreError>;
    async fn subscribe(&self) -> Result<EventStream, CoreError>;
}
```

`LocalDispatcher` wraps a `tokio::sync::broadcast` channel. Consumers subscribe and
receive a pinned stream of `AgentEvent` values. The broadcast capacity is set at
construction time (default 256 from `[core] task_queue_capacity` config).

`AgentEvent` variants: `TaskAssigned`, `TaskCompleted`, `ToolCall`, `ToolCallResult`,
`MemoryWrite`, `HookFired`, `ModelRequest`.

### 4.4 `PersonaProvider` (`crates/core/src/persona.rs`)

```rust
pub trait PersonaProvider: Send + Sync {
    fn get(&self, name: &str) -> Option<PersonaSpec>;
    fn list(&self) -> Vec<PersonaSpec>;
}
```

`PersonaSpec` holds `name`, `description`, `system_prompt`, `preferred_model`,
`allowed_tools`. `PersonaRegistry` (in `crates/personas`) loads `.toml` files from
`~/.agent007/personas/` and implements this trait. `NoOpPersonaProvider` is a stub used
before the registry is wired.

### 4.5 `VectorDB` (`crates/memory/src/vectordb/mod.rs`)

```rust
#[async_trait]
pub trait VectorDB: Send + Sync {
    async fn upsert(&self, id: &str, vector: Vec<f32>, payload: serde_json::Value)
        -> Result<(), MemoryError>;
    async fn search(&self, query: Vec<f32>, limit: usize)
        -> Result<Vec<SearchResult>, MemoryError>;
}
```

`LanceDBStore` implements this using the `lancedb = "=0.27.1"` crate. The table lives at
`~/.agent007/vectordb/`. The embedding model required to call `search` is not yet
configured (see §9).

### 4.6 `ModelRouter` (`crates/models/src/router.rs`)

Not a trait — a concrete struct. It holds a `HashMap<String, Arc<dyn ModelProvider>>` plus
routing rules and aliases. `route(task_type)` checks rules first, falls back to the
registered default. The routing table is populated from `[models.routing]` in `config.toml`
during CLI startup.

---

## 5. Memory Architecture

### 5.1 Scopes and file layout

```
~/.agent007/
  memory/
    global/          ← scope "global"
      *.json         ← one JSON file per key
    user/            ← scope "user"
    project/         ← scope "project"
    <custom>/        ← any custom namespace
  vectordb/
    agent007.lance   ← LanceDB table (arrow columnar)
  sessions/
    <run-id>/
      metadata.json
      log.jsonl
      workflow-request.json    (workflow runs only)
      workflow-state.json      (workflow runs only)
```

`MemoryStore` reads/writes individual JSON files. The filename is the URL-encoded key.
`ScopedMemoryStore` wraps `Arc<MemoryStore>` and prepends a fixed scope to all keys.

### 5.2 `MemoryEntry` metadata

Each entry JSON file carries `MemoryMeta`:

| Field | Purpose |
|-------|---------|
| `created_at`, `updated_at` | ISO-8601 timestamps |
| `access_count` | incremented on each read |
| `entry_type` | `semantic` / `procedural` / `episodic` |
| `expires_after` | optional TTL string (`"7d"`, `"24h"`) |
| `related_to` | list of related keys for 1-hop graph expansion |
| `confidence` | float 0–1; decays ×0.995 on write, +0.03 on read |
| `words` | pre-tokenized word index for RAG keyword matching |

### 5.3 LanceDB integration

`LanceDBStore` stores dense vectors alongside a JSON payload. It is used by `Retriever` for
semantic (vector) search and by `Indexer` for writing new embeddings. The embedding model
must be provided externally via `EmbeddingProvider`. This is a pending configuration
item (see §9).

---

## 6. Config Architecture

### 6.1 File locations

| File | Purpose |
|------|---------|
| `~/.agent007/config.toml` | Main config (models, routing, core settings) |
| `~/.agent007/hooks.toml` | Hook commands per event |
| `~/.agent007/personas/*.toml` | Persona definitions |
| `~/.agent007/skills/*.md` | Skill definitions |
| `~/.agent007/workflows/*.yaml` | Workflow templates |
| `$AGENT007_HOME` | Override all paths via env var |

### 6.2 Config flow

```
config.toml
  │
  ├─ [core]
  │    max_agents = 8            ─────────► Orchestrator worker pool
  │    task_queue_capacity = 256 ─────────► LocalDispatcher broadcast capacity
  │
  ├─ [models]
  │    default = "claude"        ─────────► ModelRouter default provider key
  │
  ├─ [models.routing]            ─────────► ModelRouter.add_rule(task_type, provider)
  │    code_completion = "codex"           and ModelRouter.alias(...)
  │    reasoning = "claude"
  │    fast_local = "ollama"
  │    sensitive = "ollama"
  │    default = "claude"
  │
  ├─ [models.claude]             ─────────► ClaudeProvider construction
  │    default_model = "claude-sonnet-5"
  │
  ├─ [models.codex]              ─────────► CodexProvider construction
  │    default_model = "gpt-5.3-codex"
  │
  └─ [models.ollama]             ─────────► OllamaProvider construction
       base_url = "http://localhost:11434"
       default_model = "llama3"
```

The `cli` crate reads `config.toml` at startup, constructs the providers, and calls
`ModelRouter::register` and `ModelRouter::add_rule` before handing the `Arc<ModelRouter>`
to every subsystem that needs it (workflow runner, skill executor, etc.).

### 6.3 hooks.toml flow

```
hooks.toml
  └── HookConfig::load(path)
        └── HookExecutor::new(config)
              └── HookExecutor::fire(event)
                    └── std::process::Command::new("sh").arg("-c").arg(command)
                          env: HOOK_KEY / HOOK_TOOL / HOOK_SKILL (event-specific)
```

Hooks are **synchronous** — `fire()` calls `.wait()` on the child process before returning.
This blocks the async handler for the hook's duration. See §9.

---

## 7. Error Handling Policy

| Layer | Error type | Rationale |
|-------|-----------|-----------|
| Library crates (`models`, `memory`, `skills`, `hooks`, etc.) | `thiserror`-derived typed enums | Callers can pattern-match specific variants |
| `cli` crate | `anyhow::Error` | Single binary entry point; rich context via `.context()` |
| MCP tool handlers | Return `serde_json::Value` with `"error"` field on failure | MCP protocol requires a result, not a Rust `?` propagation |

Each library crate exports its error type from `crate::error`:
`CoreError`, `ModelError`, `MemoryError`, `SkillError`, `HookError`, `WorkflowError`,
`LearningError`, `PersonaError`, `McpError`, `ZonesError`.

---

## 8. Concurrency Model

```
Tokio multi-thread runtime (default worker threads)
  │
  ├── MCP stdio handler (single-threaded message loop, rmcp)
  │     └── spawns tokio tasks per tool call when needed
  │
  ├── LocalDispatcher (tokio broadcast channel)
  │     ├── TUI subscriber (crossterm event loop)
  │     ├── WebSocket subscriber (axum ws handler, port 8007)
  │     └── RunStore log writer (appends to run-id/log.jsonl)
  │
  ├── WorkflowRunner.run() — tokio::spawn per parallel batch
  │     └── Each step: Arc<ModelProvider>.complete() (reqwest async HTTP)
  │
  └── HookExecutor.fire() — std::process::Command (sync, blocks calling task)
```

**Shared state** is always behind `Arc<T>` (or `Arc<Mutex<T>>` for interior-mutable
collections). The common pattern across crates:

```rust
Arc<dyn ModelProvider>          // model providers
Arc<dyn Dispatcher>             // event bus
Arc<dyn PersonaProvider>        // persona registry
Arc<MemoryStore>                // memory
Arc<ModelRouter>                // routing
Arc<RunStore>                   // run history
```

`async_trait` is used for `ModelProvider`, `EmbeddingProvider`, `Dispatcher`, and
`VectorDB` because these have async methods and must be object-safe (`Box<dyn Trait>`
or `Arc<dyn Trait>`).

---

## 9. Known Design Constraints and Tradeoffs

| Area | Constraint / Tradeoff |
|------|-----------------------|
| **Hook execution** | `HookExecutor::fire()` calls `std::process::Command::wait()` synchronously inside an async context. This blocks the tokio worker thread for the hook's duration. Acceptable for short scripts; a problem for slow hooks. |
| **ModelRouter ↔ skill dispatch** | `ModelRouter` routing rules are populated from config but not yet forwarded to `SkillDispatcher`. Skills call `ModelRouter::route("default")` rather than routing by `SkillFrontmatter.category`. |
| **PromptOptimizer not auto-triggered** | `FeedbackEntry` records are written on every `agent007_record_tokens` call, but the optimizer trigger (at N=20 entries per skill) is not wired. `PromptOptimizer` exists as a scaffold. |
| **LanceDB embedding model** | `LanceDBStore` implements `VectorDB` but requires an `EmbeddingProvider` to produce vectors. No concrete `EmbeddingProvider` is configured in `config.toml` yet; semantic search over memory is not active. |
| **Evaluator / Router step execution** | `StepType::Evaluator` and `StepType::Router` are fully defined in the type system and validated by `DagValidator`, but `WorkflowRunner` and `HostedWorkflowEngine` do not yet execute their conditional logic. |
| **serve.rs size** | All 44 MCP tool handlers live in a single ~4,350-line file. This is a maintainability concern but not a correctness issue. |
| **Hooks are synchronous** | The `hooks` crate uses `std::process::Command`, not `tokio::process::Command`. A future migration to async spawn would unblock the executor. |
| **Hosted-MCP workflow state** | `WorkflowRunState` is serialized to `~/.agent007/sessions/<run-id>/workflow-state.json` on every step completion. No atomic write (write then rename) — a crash mid-write could corrupt state. |
