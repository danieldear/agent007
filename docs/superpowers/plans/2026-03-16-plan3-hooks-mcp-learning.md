# agent007 Plan 3: Hooks + MCP + Learning

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `hooks` crate (event-driven shell command execution), the `mcp` crate (MCP protocol client/server via official Rust MCP SDK), and the `learning` crate (feedback collection, reward scoring, prompt optimization) so that agent007 can fire lifecycle hooks, connect to external MCP tool servers, and self-improve its prompts over time.

**Architecture:** Three independent library crates. `hooks` is self-contained (no crate deps beyond workspace). `mcp` wraps the official Rust MCP SDK and is also self-contained. `learning` depends on `core` (subscribes to `AgentEvent` via `Dispatcher`), `memory` (for `ScopedMemoryStore` + LanceDB), `models` (for `ModelProvider` to call the optimizer model), and `skills` (to read/write skill versions). `learning` owns its own `LearningDispatcher` instance — a `LocalDispatcher` parameterized on `LearningEvent` — so the TUI can subscribe to learning events without any circular dependency with `core`.

**Tech Stack:** Rust, tokio (full features), async-trait, serde/serde_json, toml, thiserror, tracing, uuid, chrono, tokio-stream, official Rust MCP SDK (pre-build: confirm crate name + version on crates.io — may be `rmcp`, `mcp-sdk`, or `modelcontextprotocol`)

**Prerequisites:** Plans 1 and 2 complete. `crates/models`, `crates/core`, `crates/memory`, and `crates/skills` exist and build.

**Spec:** `docs/superpowers/specs/2026-03-16-agent007-design.md`

---

## Chunk 1: hooks crate

### File Structure (Chunk 1)

```
crates/hooks/
├── Cargo.toml
└── src/
    ├── lib.rs         # pub re-exports: HookConfig, HookEvent, HookExecutor, HookError
    ├── error.rs       # HookError (thiserror)
    ├── config.rs      # HookConfig — TOML schema for ~/.agent007/hooks/hooks.toml
    └── executor.rs    # HookExecutor — fires shell commands via std::process::Command
```

---

### Task 1: hooks crate bootstrap + HookConfig + HookEvent

**Files:**
- Create: `crates/hooks/Cargo.toml`
- Create: `crates/hooks/src/lib.rs`
- Create: `crates/hooks/src/error.rs`
- Create: `crates/hooks/src/config.rs`
- Modify: `Cargo.toml` (workspace root — add `crates/hooks` to members)

- [ ] **Step 1: Add hooks to workspace**

Add `"crates/hooks"` to the `members` list in the workspace root `Cargo.toml`. Also add `toml` to `[workspace.dependencies]` if not already present:

```toml
# In [workspace.dependencies] of root Cargo.toml — add if missing:
toml = "0.8"
```

- [ ] **Step 2: Create hooks Cargo.toml**

```toml
# crates/hooks/Cargo.toml
[package]
name = "agent007-hooks"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
toml = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
tempfile = { workspace = true }
```

- [ ] **Step 3: Create lib.rs**

```rust
// crates/hooks/src/lib.rs
pub mod config;
pub mod error;
pub mod executor;

pub use config::{HookConfig, HookEvent};
pub use error::HookError;
pub use executor::HookExecutor;
```

- [ ] **Step 4: Write failing tests for HookConfig parsing**

In `crates/hooks/src/config.rs`, add a `#[cfg(test)]` module only (no struct definitions yet):

```rust
// crates/hooks/src/config.rs — tests only, structs added in Step 5
#[cfg(test)]
mod tests {
    // test: parse a minimal hooks.toml, verify all fields parse correctly
    // test: parse hooks.toml with empty string values, verify Option::Some("") or Option::None as designed
    // test: HookEvent variants serialize to expected string names
}
```

- [ ] **Step 5: Implement HookConfig and HookEvent**

`HookConfig` struct with fields (sketch — no full body):

```rust
// crates/hooks/src/config.rs
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct HookConfig {
    pub pre_agent_run: Option<String>,
    pub post_agent_run: Option<String>,
    pub pre_tool_call: Option<String>,
    pub post_tool_call: Option<String>,
    pub on_memory_write: Option<String>,
    pub on_skill_execute: Option<String>,
    pub post_task_complete: Option<String>,
}

impl HookConfig {
    /// Load from ~/.agent007/hooks/hooks.toml. Returns default (all None) if file absent.
    pub fn load(path: &std::path::Path) -> Result<Self, crate::error::HookError> { ... }

    /// Resolve the shell command string for a given event. Returns None if not configured or empty.
    pub fn command_for(&self, event: &HookEvent) -> Option<&str> { ... }
}

#[derive(Debug, Clone)]
pub enum HookEvent {
    PreAgentRun,
    PostAgentRun,
    PreToolCall { tool: String },
    PostToolCall { tool: String },
    OnMemoryWrite { key: String },
    OnSkillExecute { skill: String },
    PostTaskComplete,
}
```

- [ ] **Step 6: Implement HookError**

```rust
// crates/hooks/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("failed to read hooks config at {path}: {source}")]
    ConfigRead { path: std::path::PathBuf, source: std::io::Error },

    #[error("failed to parse hooks.toml: {0}")]
    ConfigParse(#[from] toml::de::Error),

    #[error("hook command failed with exit code {code}: {command}")]
    CommandFailed { command: String, code: i32 },

    #[error("failed to spawn hook command '{command}': {source}")]
    SpawnFailed { command: String, source: std::io::Error },
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p agent007-hooks config::tests
```

Expected: all config-parsing tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/hooks/ Cargo.toml
git commit -m "feat(hooks): add HookConfig, HookEvent, HookError types"
```

---

### Task 2: HookExecutor

**Files:**
- Create: `crates/hooks/src/executor.rs`

- [ ] **Step 1: Write failing tests**

In `crates/hooks/src/executor.rs`, add `#[cfg(test)]` module only:

```rust
// crates/hooks/src/executor.rs — tests only, struct added in Step 2
#[cfg(test)]
mod tests {
    // test: fire PreAgentRun with an "echo hello" command; verify returns Ok(())
    // test: fire PostAgentRun with None/empty command string; verify no-op returns Ok(())
    // test: fire PreAgentRun with "exit 1" command; verify HookError::CommandFailed returned
    // test: fire OnMemoryWrite with key="testkey"; verify command receives key in env or args as configured
}
```

- [ ] **Step 2: Implement HookExecutor**

```rust
// crates/hooks/src/executor.rs
pub struct HookExecutor {
    config: crate::config::HookConfig,
}

impl HookExecutor {
    pub fn new(config: crate::config::HookConfig) -> Self { ... }

    /// Fire the hook for the given event. If no command is configured, returns Ok(()) immediately.
    /// Spawns the shell command via std::process::Command, waits for exit, returns error on non-zero.
    pub fn fire(&self, event: &crate::config::HookEvent) -> Result<(), crate::error::HookError> { ... }
}
```

Shell command execution notes:
- Use `std::process::Command::new("sh").arg("-c").arg(command)` to allow shell syntax in hook strings.
- On non-zero exit code, return `HookError::CommandFailed { command, code }`.
- On spawn failure (binary not found, permissions), return `HookError::SpawnFailed { command, source }`.
- `tracing::debug!` the event and command before firing; `tracing::warn!` on failure.

- [ ] **Step 3: Run tests**

```bash
cargo test -p agent007-hooks executor::tests
```

Expected: all HookExecutor tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/hooks/src/executor.rs
git commit -m "feat(hooks): add HookExecutor — fires shell commands on agent lifecycle events"
```

---

## Chunk 2: mcp crate

### File Structure (Chunk 2)

```
crates/mcp/
├── Cargo.toml
└── src/
    ├── lib.rs         # pub re-exports: McpServerConfig, McpClient, McpError, ToolDef
    ├── error.rs       # McpError (thiserror)
    ├── config.rs      # McpServerConfig — matches [mcp.servers] TOML section
    └── client.rs      # McpClient — wraps official Rust MCP SDK
```

---

### Task 3: mcp crate bootstrap + McpServerConfig + McpError

**Files:**
- Create: `crates/mcp/Cargo.toml`
- Create: `crates/mcp/src/lib.rs`
- Create: `crates/mcp/src/error.rs`
- Create: `crates/mcp/src/config.rs`
- Modify: `Cargo.toml` (workspace root — add `crates/mcp` to members and MCP SDK dep)

- [ ] **Step 1: PRE-BUILD — confirm MCP SDK crate name and version**

Before writing any code, check crates.io for the official Rust MCP SDK:

```bash
# Check which name is published
cargo search rmcp
cargo search mcp-sdk
cargo search modelcontextprotocol
```

Pin the confirmed crate name and exact version in the workspace `Cargo.toml` before proceeding. The spec notes the crate may be `rmcp`, `mcp-sdk`, or `modelcontextprotocol`. Use whichever is current. Example (fill in confirmed name/version):

```toml
# In [workspace.dependencies] of root Cargo.toml — add after confirming:
rmcp = { version = "X.Y.Z", features = ["client"] }  # replace with confirmed name + version
```

- [ ] **Step 2: Add mcp to workspace and create Cargo.toml**

Add `"crates/mcp"` to the workspace members list. Create:

```toml
# crates/mcp/Cargo.toml
[package]
name = "agent007-mcp"
version = "0.1.0"
edition = "2021"

[dependencies]
rmcp = { workspace = true }         # replace with confirmed SDK crate name
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
tokio = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

- [ ] **Step 3: Create lib.rs**

```rust
// crates/mcp/src/lib.rs
pub mod config;
pub mod error;
pub mod client;

pub use config::McpServerConfig;
pub use error::McpError;
pub use client::{McpClient, ToolDef};
```

- [ ] **Step 4: Write failing tests for McpServerConfig**

In `crates/mcp/src/config.rs`, add a `#[cfg(test)]` module only:

```rust
// crates/mcp/src/config.rs — tests only, struct added in Step 5
#[cfg(test)]
mod tests {
    // test: parse a [mcp.servers] TOML table with two entries (filesystem, github)
    //       verify McpServerConfig fields (name, command) populated correctly
    // test: empty servers section deserializes to empty Vec
}
```

- [ ] **Step 5: Implement McpServerConfig and McpError**

```rust
// crates/mcp/src/config.rs
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct McpServerConfig {
    /// Human-readable name (e.g., "filesystem", "github")
    pub name: String,
    /// Shell command to start the MCP server subprocess (e.g., "npx @modelcontextprotocol/server-filesystem")
    pub command: String,
}
```

```rust
// crates/mcp/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("failed to start MCP server '{name}': {source}")]
    ServerStartFailed { name: String, source: std::io::Error },

    #[error("MCP SDK error: {0}")]
    Sdk(String),

    #[error("tool not found: {0}")]
    ToolNotFound(String),

    #[error("tool call failed for '{tool}': {reason}")]
    ToolCallFailed { tool: String, reason: String },

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p agent007-mcp config::tests
```

Expected: all config tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/mcp/ Cargo.toml
git commit -m "feat(mcp): add McpServerConfig and McpError types"
```

---

### Task 4: McpClient

**Files:**
- Create: `crates/mcp/src/client.rs`

- [ ] **Step 1: Write failing tests**

In `crates/mcp/src/client.rs`, add `#[cfg(test)]` module only:

```rust
// crates/mcp/src/client.rs — tests only, struct added in Step 2
#[cfg(test)]
mod tests {
    // NOTE: per spec, test against a local rmcp test server — do NOT use npx subprocess in CI
    // test: McpClient::new() with an empty server list initializes without error
    // test: list_tools() on a client connected to a local rmcp echo test server returns at least one ToolDef
    // test: call_tool() on the echo tool returns expected JSON response
    // test: call_tool() with unknown tool name returns McpError::ToolNotFound
}
```

- [ ] **Step 2: Implement McpClient and ToolDef**

```rust
// crates/mcp/src/client.rs
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

pub struct McpClient {
    servers: Vec<crate::config::McpServerConfig>,
    // internal: connected SDK handles, one per server
}

impl McpClient {
    /// Create client from server configs. Does not start servers yet.
    pub fn new(servers: Vec<crate::config::McpServerConfig>) -> Self { ... }

    /// Start all configured server subprocesses and connect via the MCP SDK.
    pub async fn connect(&mut self) -> Result<(), crate::error::McpError> { ... }

    /// Return all tools advertised by all connected servers.
    pub async fn list_tools(&self) -> Result<Vec<ToolDef>, crate::error::McpError> { ... }

    /// Call a named tool with JSON args. Returns the tool's JSON response.
    pub async fn call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, crate::error::McpError> { ... }
}
```

Implementation notes:
- Use the confirmed MCP SDK's client API to spawn each server as a subprocess and establish the MCP connection.
- `list_tools()` aggregates `ToolDef` entries from all connected servers; deduplicate by name (last-wins if conflict).
- `call_tool()` routes to whichever server advertised the named tool; returns `McpError::ToolNotFound` if absent.
- `tracing::info!` each server connect and tool call; `tracing::error!` on failures.

- [ ] **Step 3: Run tests**

```bash
cargo test -p agent007-mcp client::tests
```

Expected: all McpClient tests pass (using local test server, no npx).

- [ ] **Step 4: Commit**

```bash
git add crates/mcp/src/client.rs
git commit -m "feat(mcp): add McpClient — wraps official Rust MCP SDK for tool listing and calling"
```

---

## Chunk 3: learning crate

### File Structure (Chunk 3)

```
crates/learning/
├── Cargo.toml
└── src/
    ├── lib.rs            # pub re-exports
    ├── error.rs          # LearningError (thiserror)
    ├── types.rs          # FeedbackEntry, Outcome, LearningEvent
    ├── store.rs          # LearningStore — persists to ScopedMemoryStore("learning") + LanceDB
    ├── collector.rs      # FeedbackCollector — subscribes to core Dispatcher
    ├── scorer.rs         # RewardScorer — assigns f32 reward in [-1, +1]
    └── optimizer.rs      # PromptOptimizer — rewrites skill prompts via reasoning model
```

---

### Task 5: learning crate bootstrap + types

**Files:**
- Create: `crates/learning/Cargo.toml`
- Create: `crates/learning/src/lib.rs`
- Create: `crates/learning/src/error.rs`
- Create: `crates/learning/src/types.rs`
- Modify: `Cargo.toml` (workspace root — add `crates/learning` to members)

- [ ] **Step 1: Add learning to workspace**

Add `"crates/learning"` to the workspace members list.

- [ ] **Step 2: Create learning Cargo.toml**

```toml
# crates/learning/Cargo.toml
[package]
name = "agent007-learning"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core    = { path = "../core" }
agent007-memory  = { path = "../memory" }
agent007-models  = { path = "../models" }
agent007-skills  = { path = "../skills" }
async-trait = { workspace = true }
futures     = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
tracing     = { workspace = true }
tokio       = { workspace = true }
tokio-stream = { workspace = true }
uuid        = { workspace = true }
chrono      = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

- [ ] **Step 3: Create lib.rs and stub files for not-yet-implemented modules**

```rust
// crates/learning/src/lib.rs
pub mod collector;
pub mod dispatcher;
pub mod error;
pub mod optimizer;
pub mod scorer;
pub mod store;
pub mod types;

pub use error::LearningError;
pub use types::{FeedbackEntry, LearningEvent, Outcome};
pub use collector::FeedbackCollector;
pub use dispatcher::LearningDispatcher;
pub use scorer::RewardScorer;
pub use optimizer::PromptOptimizer;
pub use store::LearningStore;
```

Create empty stub files for all modules declared in lib.rs that are not yet implemented, so the crate compiles at each intermediate step:

```bash
touch crates/learning/src/collector.rs \
      crates/learning/src/dispatcher.rs \
      crates/learning/src/optimizer.rs \
      crates/learning/src/scorer.rs \
      crates/learning/src/store.rs
```

Each stub file starts empty — the implementer fills it in at the appropriate task.

- [ ] **Step 4: Write failing tests for types**

In `crates/learning/src/types.rs`, add `#[cfg(test)]` module only:

```rust
// crates/learning/src/types.rs — tests only, types added in Step 5
#[cfg(test)]
mod tests {
    // test: FeedbackEntry serializes to JSON and round-trips correctly
    // test: Outcome::Success serializes; Outcome::Failure deserializes with reason field
    // test: LearningEvent::PromptImproved carries old_reward and new_reward as f32
}
```

- [ ] **Step 5: Implement FeedbackEntry, Outcome, LearningEvent**

```rust
// crates/learning/src/types.rs
use chrono::{DateTime, Utc};
use uuid::Uuid;
use agent007_core::types::{AgentId, PromptRef};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FeedbackEntry {
    pub id: Uuid,
    pub agent_id: AgentId,
    pub prompt_ref: PromptRef,
    pub skill_name: Option<String>,
    pub model: String,
    pub outcome: Outcome,
    pub reward: Option<f32>,       // set after scoring; None until RewardScorer runs
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Outcome {
    Success,
    Failure { reason: String },
    UserRating { score: f32 },     // explicit thumbs-up/down from TUI
    ToolError { tool: String },
}

// LearningEvent is entirely separate from AgentEvent — avoids circular dep with core.
// learning crate owns its own LocalDispatcher<LearningEvent>.
#[derive(Debug, Clone)]
pub enum LearningEvent {
    PromptImproved { skill_name: String, old_reward: f32, new_reward: f32 },
    FeedbackRecorded { agent_id: AgentId, reward: f32 },
    OptimizerTriggered { skill_name: String },
}
```

- [ ] **Step 6: Implement LearningError**

```rust
// crates/learning/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum LearningError {
    #[error("memory error: {0}")]
    Memory(#[from] agent007_memory::error::MemoryError),

    #[error("model error: {0}")]
    Model(#[from] agent007_models::ModelError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("feedback entry not found: {0}")]
    NotFound(uuid::Uuid),

    #[error("optimizer failed for skill '{skill}': {reason}")]
    OptimizerFailed { skill: String, reason: String },

    #[error("dispatcher error: {0}")]
    Dispatcher(String),
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p agent007-learning types::tests
```

Expected: all FeedbackEntry/Outcome/LearningEvent type tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/learning/ Cargo.toml
git commit -m "feat(learning): add FeedbackEntry, Outcome, LearningEvent types and LearningError"
```

---

### Task 6: LearningStore

**Files:**
- Create: `crates/learning/src/store.rs`

- [ ] **Step 1: Write failing tests**

In `crates/learning/src/store.rs`, add `#[cfg(test)]` module only:

```rust
// crates/learning/src/store.rs — tests only, struct added in Step 2
#[cfg(test)]
mod tests {
    // test: LearningStore::new() creates a scoped store under "learning" namespace
    // test: record_feedback() persists a FeedbackEntry and get_entry() retrieves it by id
    // test: list_recent_feedback(skill_name, n) returns at most n entries for the given skill
    // test: save_prompt_version() stores a new version; get_prompt_versions() returns all versions in order
    // test: get_prompt_versions() returns empty list for unknown skill
}
```

- [ ] **Step 2: Implement LearningStore**

```rust
// crates/learning/src/store.rs
use agent007_memory::store::ScopedMemoryStore;

pub struct LearningStore {
    // Uses MemoryStore::scoped("learning") — all keys prefixed with "learning/"
    scoped: ScopedMemoryStore,
}

pub struct PromptVersion {
    pub version: u32,
    pub skill_name: String,
    pub prompt_text: String,
    pub avg_reward: Option<f32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl LearningStore {
    /// Construct from a ScopedMemoryStore already scoped to "learning".
    pub fn new(scoped: ScopedMemoryStore) -> Self { ... }

    /// Persist a FeedbackEntry (serialized as JSON) under key "feedback/<entry.id>".
    pub fn record_feedback(&self, entry: &crate::types::FeedbackEntry)
        -> Result<(), crate::error::LearningError> { ... }

    /// Retrieve a FeedbackEntry by id.
    pub fn get_entry(&self, id: uuid::Uuid)
        -> Result<Option<crate::types::FeedbackEntry>, crate::error::LearningError> { ... }

    /// List the N most recent FeedbackEntry records for a given skill_name.
    pub fn list_recent_feedback(&self, skill_name: &str, n: usize)
        -> Result<Vec<crate::types::FeedbackEntry>, crate::error::LearningError> { ... }

    /// Save an improved prompt as a new version for the given skill.
    pub fn save_prompt_version(&self, version: PromptVersion)
        -> Result<(), crate::error::LearningError> { ... }

    /// Return all saved PromptVersion records for a skill, ordered by version ascending.
    pub fn get_prompt_versions(&self, skill_name: &str)
        -> Result<Vec<PromptVersion>, crate::error::LearningError> { ... }
}
```

Storage layout inside the `ScopedMemoryStore("learning")`:
- Feedback entries: key `feedback/<uuid>`
- Prompt versions: key `versions/<skill_name>/<version_number>`

- [ ] **Step 3: Run tests**

```bash
cargo test -p agent007-learning store::tests
```

Expected: all LearningStore tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/learning/src/store.rs
git commit -m "feat(learning): add LearningStore — persists FeedbackEntry and prompt versions"
```

---

### Task 7: FeedbackCollector

**Files:**
- Create: `crates/learning/src/collector.rs`

- [ ] **Step 1: Write failing tests**

In `crates/learning/src/collector.rs`, add `#[cfg(test)]` module only:

```rust
// crates/learning/src/collector.rs — tests only, struct added in Step 2
#[cfg(test)]
mod tests {
    // Setup: create a mock Dispatcher (hand-written, same pattern as MockProvider in models crate)
    // test: FeedbackCollector::new() accepts an Arc<dyn Dispatcher> and a LearningStore
    // test: on receiving AgentEvent::TaskCompleted, collector creates a FeedbackEntry with
    //       Outcome::Success or Outcome::Failure, passes it to RewardScorer, persists via LearningStore
    // test: on receiving AgentEvent::ToolCall, collector creates a FeedbackEntry with
    //       Outcome::ToolError if the tool result indicates error, Outcome::Success otherwise
    // test: events not relevant to learning (e.g., HookFired) are silently ignored
}
```

- [ ] **Step 2: Implement FeedbackCollector**

```rust
// crates/learning/src/collector.rs
use std::sync::Arc;
use agent007_core::dispatcher::Dispatcher;

pub struct FeedbackCollector {
    dispatcher: Arc<dyn Dispatcher>,
    store: crate::store::LearningStore,
    scorer: crate::scorer::RewardScorer,
}

impl FeedbackCollector {
    pub fn new(
        dispatcher: Arc<dyn Dispatcher>,
        store: crate::store::LearningStore,
        scorer: crate::scorer::RewardScorer,
    ) -> Self { ... }

    /// Subscribe to the core Dispatcher and process AgentEvents in a loop.
    /// Call this in a spawned tokio task. Returns when the stream ends or cancellation is signalled.
    pub async fn run(&self) -> Result<(), crate::error::LearningError> { ... }
}
```

Implementation notes:
- Call `self.dispatcher.subscribe()` to get the `Pin<Box<dyn Stream<Item = AgentEvent>>>`.
- Match on `AgentEvent::TaskCompleted` and `AgentEvent::ToolCall`; ignore all other variants.
- For each matched event: construct a `FeedbackEntry`, call `self.scorer.score(&entry)` to set `entry.reward`, persist via `self.store.record_feedback(&entry)`.
- `tracing::debug!` each entry recorded; `tracing::warn!` on store/scorer errors (do not propagate — collector must stay alive).

- [ ] **Step 3: Run tests**

```bash
cargo test -p agent007-learning collector::tests
```

Expected: all FeedbackCollector tests pass with mock dispatcher.

- [ ] **Step 4: Commit**

```bash
git add crates/learning/src/collector.rs
git commit -m "feat(learning): add FeedbackCollector — subscribes to core Dispatcher and records feedback"
```

---

### Task 8: RewardScorer

**Files:**
- Create: `crates/learning/src/scorer.rs`

- [ ] **Step 1: Write failing tests**

In `crates/learning/src/scorer.rs`, add `#[cfg(test)]` module only:

```rust
// crates/learning/src/scorer.rs — tests only, struct added in Step 2
#[cfg(test)]
mod tests {
    // test: score() for Outcome::Success with no user rating, no tool errors, no retries
    //       returns task_completion * 0.4 = +0.4
    // test: score() for Outcome::Failure returns task_completion * 0.4 = -0.4
    // test: score() with UserRating{score: 1.0} adds +0.3 to signal
    // test: score() with UserRating{score: -1.0} adds -0.3 to signal
    // test: score() with tool_error_count=2, total_tool_calls=4 subtracts 0.2*(2/4) = -0.1
    // test: score() with retry_count=5, max_retries=5 subtracts full 0.1 penalty
    // test: score() result always clamped to [-1.0, +1.0] — never exceeds bounds
    // test: score() when only Outcome signal present (no user rating, no tool data):
    //       remaining weights renormalized so completion weight = 1.0; result = ±1.0
}
```

- [ ] **Step 2: Implement RewardScorer**

```rust
// crates/learning/src/scorer.rs
pub struct RewardWeights {
    pub completion: f32,   // default 0.4
    pub user_rating: f32,  // default 0.3
    pub tool_errors: f32,  // default 0.2
    pub retries: f32,      // default 0.1
}

impl Default for RewardWeights { ... }

pub struct ScoringContext {
    pub outcome: crate::types::Outcome,
    pub user_rating: Option<f32>,       // +1.0 / -1.0 / None
    pub tool_error_count: Option<u32>,
    pub total_tool_calls: Option<u32>,
    pub retry_count: Option<u32>,
    pub max_retries: Option<u32>,
}

pub struct RewardScorer {
    weights: RewardWeights,
}

impl RewardScorer {
    pub fn new(weights: RewardWeights) -> Self { ... }

    /// Compute a scalar reward in [-1.0, +1.0] from the scoring context.
    /// Signals with no data are omitted; remaining weights are renormalized.
    pub fn score(&self, ctx: &ScoringContext) -> f32 { ... }
}
```

Scoring algorithm (pseudocode):
1. Collect (signal_value, weight) pairs for each present signal.
2. Normalize: total_weight = sum of present weights; scaled = signal * (weight / total_weight).
3. Sum all scaled values.
4. Clamp result to [-1.0, +1.0].

Signal values:
- task_completion: +1.0 if `Outcome::Success | Outcome::UserRating`, -1.0 if `Outcome::Failure | Outcome::ToolError`
- user_rating: present only when `Outcome::UserRating { score }` — use score directly (already in [-1, +1])
- tool_error rate: `-(error_count / total_calls)` — present only when tool_calls data available
- retry penalty: `-(retry_count / max_retries).min(1.0)` — present only when retry data available

- [ ] **Step 3: Run tests**

```bash
cargo test -p agent007-learning scorer::tests
```

Expected: all RewardScorer tests pass; bounds and renormalization verified.

- [ ] **Step 4: Commit**

```bash
git add crates/learning/src/scorer.rs
git commit -m "feat(learning): add RewardScorer — weighted reward signals in [-1, +1] with renormalization"
```

---

### Task 9: LearningDispatcher

**Note on ordering:** `LearningDispatcher` is implemented before `PromptOptimizer` (Task 10) because `PromptOptimizer` holds an `Arc<crate::dispatcher::LearningDispatcher>`. The module must exist before the optimizer can reference it.

**Files:**
- Modify: `crates/learning/src/dispatcher.rs` (was empty stub from Task 5; fill in implementation)

The `learning` crate owns its own `LocalDispatcher` instance parameterized on `LearningEvent`. This mirrors the `LocalDispatcher` in `core` but is a separate instance — no circular dependency. The TUI subscribes to both `core`'s `Dispatcher<AgentEvent>` and `learning`'s `LearningDispatcher` independently.

- [ ] **Step 1: Write failing tests**

In `crates/learning/src/dispatcher.rs`, add `#[cfg(test)]` module only:

```rust
// crates/learning/src/dispatcher.rs — tests only
#[cfg(test)]
mod tests {
    // test: LearningDispatcher::new() creates a working publish/subscribe channel
    // test: publish(LearningEvent::FeedbackRecorded{..}) is received by a subscriber
    // test: publish(LearningEvent::OptimizerTriggered{..}) is received; unrelated subscribers see it too
    // test: subscriber receives events in order published
}
```

- [ ] **Step 2: Implement LearningDispatcher**

`pub mod dispatcher;` and `pub use dispatcher::LearningDispatcher;` are already declared in `lib.rs` (added in Task 5 Step 3 alongside the other stub declarations). Implement the file contents now:

```rust
// crates/learning/src/dispatcher.rs
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use std::pin::Pin;

pub struct LearningDispatcher {
    sender: broadcast::Sender<crate::types::LearningEvent>,
}

impl LearningDispatcher {
    pub fn new(capacity: usize) -> Self { ... }

    pub fn publish(&self, event: crate::types::LearningEvent) -> Result<(), crate::error::LearningError> { ... }

    pub fn subscribe(&self) -> Pin<Box<dyn futures::Stream<Item = crate::types::LearningEvent> + Send>> { ... }
}
```

Implementation notes:
- Backed by `tokio::sync::broadcast` channel with the given capacity (default 1024, same as core).
- `subscribe()` wraps `self.sender.subscribe()` in a `BroadcastStream` (from `tokio-stream`) and boxes it — same pattern as `core`'s `LocalDispatcher`.
- `LearningEvent` must derive `Clone` (required by broadcast channels).

- [ ] **Step 3: Run tests**

```bash
cargo test -p agent007-learning dispatcher::tests
```

Expected: all LearningDispatcher tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/learning/src/dispatcher.rs crates/learning/src/lib.rs
git commit -m "feat(learning): add LearningDispatcher — own event bus for LearningEvent, separate from core"
```

---

### Task 10: PromptOptimizer

**Files:**
- Create: `crates/learning/src/optimizer.rs`

- [ ] **Step 1: Write failing tests**

In `crates/learning/src/optimizer.rs`, add `#[cfg(test)]` module only:

```rust
// crates/learning/src/optimizer.rs — tests only, struct added in Step 2
#[cfg(test)]
mod tests {
    // Setup: use MockProvider (from agent007-models) as the reasoning model
    //        configure it to return a canned "improved prompt text"
    // test: maybe_optimize() when avg_reward >= threshold (e.g., 0.5) does NOT call model or store new version
    // test: maybe_optimize() when avg_reward < threshold (e.g., 0.2) AND entry count >= trigger_count:
    //       calls ModelProvider once with meta-prompt containing failure examples,
    //       stores new PromptVersion in LearningStore, emits OptimizerTriggered LearningEvent
    // test: maybe_optimize() when entry count < trigger_count does NOT trigger optimization
    // test: saved prompt version number increments correctly on successive optimizations
}
```

- [ ] **Step 2: Implement PromptOptimizer**

```rust
// crates/learning/src/optimizer.rs
use std::sync::Arc;
use agent007_models::provider::ModelProvider;

pub struct OptimizerConfig {
    pub threshold: f32,          // default 0.3 — avg reward below this triggers optimization
    pub trigger_count: usize,    // default 10 — minimum feedback entries before optimizer runs
    pub optimizer_model: String, // default "claude"
}

impl Default for OptimizerConfig { ... }

pub struct PromptOptimizer {
    config: OptimizerConfig,
    provider: Arc<dyn ModelProvider>,
    learning_dispatcher: Arc<crate::dispatcher::LearningDispatcher>,
}

impl PromptOptimizer {
    pub fn new(
        config: OptimizerConfig,
        provider: Arc<dyn ModelProvider>,
        learning_dispatcher: Arc<crate::dispatcher::LearningDispatcher>,
    ) -> Self { ... }

    /// Check whether optimization should run for the given skill.
    /// If avg reward of recent entries is below threshold and count >= trigger_count:
    ///   1. Retrieve original prompt from skills store (via skills_store param)
    ///   2. Retrieve top-k similar failed FeedbackEntry records via LanceDB RAG search
    ///      (similarity search on the feedback namespace, filtered to Failure/ToolError outcomes)
    ///   3. Call provider with meta-prompt to rewrite the skill prompt
    ///   4. Save new PromptVersion in LearningStore (original preserved)
    ///   5. Emit LearningEvent::OptimizerTriggered via LearningDispatcher
    pub async fn maybe_optimize(
        &self,
        skill_name: &str,
        store: &crate::store::LearningStore,
        skills_store: &agent007_skills::SkillStore,
    ) -> Result<(), crate::error::LearningError> { ... }
}
```

Meta-prompt template (inline string constant — no Tera needed here; use Rust `format!` to fill placeholders):

```
You are a prompt engineer. The following skill prompt has been producing
poor results (average reward: {avg_reward:.2}). Here are examples of its
failures:

{failure_examples}

Rewrite the prompt to fix these failure patterns. Keep the same goal.
Return only the improved prompt text.
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p agent007-learning optimizer::tests
```

Expected: all PromptOptimizer tests pass with MockProvider.

- [ ] **Step 4: Commit all learning crate work**

```bash
git add crates/learning/src/optimizer.rs
git commit -m "feat(learning): add PromptOptimizer — triggers meta-prompt rewrite when avg reward below threshold"
```

---

## Summary

After all chunks are complete, the following crates are implemented and tested:

| Crate | Package name | Key types |
|---|---|---|
| `crates/hooks` | `agent007-hooks` | `HookConfig`, `HookEvent`, `HookExecutor`, `HookError` |
| `crates/mcp` | `agent007-mcp` | `McpServerConfig`, `McpClient`, `ToolDef`, `McpError` |
| `crates/learning` | `agent007-learning` | `FeedbackEntry`, `Outcome`, `LearningEvent`, `LearningStore`, `FeedbackCollector`, `RewardScorer`, `LearningDispatcher`, `PromptOptimizer`, `LearningError` |

**Dependency graph for this plan:**

```
hooks       (no crate deps — stdlib + serde + toml only)
mcp         (no crate deps — stdlib + MCP SDK only)
learning    → core (AgentEvent, Dispatcher, AgentId, PromptRef)
            → memory (ScopedMemoryStore, MemoryStore)
            → models (ModelProvider, MockProvider in tests)
            → skills (SkillStore for reading/writing skill prompts)
```

**Key spec constraints satisfied:**

- `HookExecutor` fires shell commands via `sh -c` — allows shell syntax in hook strings.
- `HookEvent` variants match the seven events listed in the spec exactly.
- MCP SDK crate name is verified on crates.io pre-build and pinned to an exact version.
- `McpClient` acts as both MCP client (consumes tool servers) and is designed to expose agent007 tools as an MCP server (server-side implementation deferred to Phase 2 CLI wiring).
- `LearningEvent` is entirely separate from `AgentEvent` — no circular import with `core`.
- `LearningDispatcher` implemented before `PromptOptimizer` — dependency ordering correct within Chunk 3.
- `LearningDispatcher` is a standalone `broadcast`-backed instance in the `learning` crate — TUI subscribes to it independently from `core`'s dispatcher.
- `RewardScorer` weights match spec exactly: completion=0.4, user_rating=0.3, tool_errors=0.2, retries=0.1; output clamped to [-1.0, +1.0]; missing signals trigger renormalization.
- `PromptOptimizer` step 2 uses RAG/LanceDB similarity search for failed entries (per spec), not just a simple list lookup.
- `PromptOptimizer` triggers at configurable threshold (default 0.3) and trigger_count (default 10); uses `ModelProvider` so tests use `MockProvider` — no real API calls.
- `futures` crate added to `learning` Cargo.toml deps — required by `LearningDispatcher::subscribe()` return type.
- All library crates use `thiserror`; no `anyhow` in any crate except `cli`.
- All async traits use `async-trait = "0.1"` for object safety (consistent with `core` and `models` policy).
