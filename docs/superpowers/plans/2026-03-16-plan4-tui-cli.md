# agent007 Plan 4: TUI + CLI Wiring

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the ratatui TUI dashboard and complete the CLI binary that wires all Phase 1 crates together, making agent007 fully operational end-to-end.

**Architecture:** The `tui` crate owns all terminal rendering logic — it subscribes independently to the core `Dispatcher` (for `AgentEvent`s) and the learning `LearningDispatcher` (for `LearningEvent`s), avoiding circular imports between `core` and `learning`. The `cli` crate is the final integration layer: it parses subcommands via clap, loads `~/.agent007/config.toml` into `Arc<Config>`, constructs every Phase 1 subsystem (`LocalDispatcher`, `ModelRouter`, `MemoryStore`, `Retriever`, `Indexer`, `SkillLoader`, `SkillExecutor`, `HookExecutor`, `McpClient`, `FeedbackCollector`, `LearningDispatcher`), spawns the `Orchestrator` with the user task, and hands control to the TUI event loop.

**Tech Stack:** ratatui, crossterm, clap (derive), tokio, tokio-stream, tokio-util (CancellationToken/TaskTracker), futures, agent007-core, agent007-models, agent007-memory, agent007-skills, agent007-hooks, agent007-mcp, agent007-learning, thiserror (tui crate), anyhow (cli only), serde/serde_json, toml, tracing

**Prerequisites:** Plans 1, 2, 3 complete. All library crates built and tested.

**Spec:** `docs/superpowers/specs/2026-03-16-agent007-design.md`

---

## Chunk 1: tui crate

### File Structure (Chunk 1)

```
crates/tui/
├── Cargo.toml
└── src/
    ├── lib.rs          # pub re-exports: App, EventLoop, TuiError
    ├── error.rs        # TuiError (thiserror)
    ├── app.rs          # App state — subscribes to Dispatcher + LearningDispatcher
    ├── ui.rs           # render(frame, app) — draws all 6 panels
    └── event.rs        # EventLoop — crossterm keyboard + event stream fan-in
```

---

### Task 1: tui crate bootstrap

**Files:**
- Create: `crates/tui/Cargo.toml`
- Create: `crates/tui/src/lib.rs`
- Create: `crates/tui/src/error.rs`
- Modify: `Cargo.toml` (workspace root — add `crates/tui` to members; add ratatui, crossterm)

- [ ] **Step 1: Add tui to workspace and add new workspace deps**

Add `"crates/tui"` to `Cargo.toml` members list. Add to `[workspace.dependencies]`:

```toml
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
toml = "0.8"
```

- [ ] **Step 2: Create tui Cargo.toml**

```toml
# crates/tui/Cargo.toml
[package]
name = "agent007-tui"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core = { path = "../core" }
agent007-learning = { path = "../learning" }
ratatui = { workspace = true }
crossterm = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tokio-stream = { workspace = true }
futures = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

- [ ] **Step 3: Create error.rs**

```rust
// crates/tui/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("terminal IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("event bus subscribe error: {0}")]
    Subscribe(String),

    #[error("crossterm error: {0}")]
    Crossterm(String),
}
```

- [ ] **Step 4: Create lib.rs skeleton**

```rust
// crates/tui/src/lib.rs
pub mod error;
pub mod app;
pub mod ui;
pub mod event;

pub use error::TuiError;
pub use app::App;
pub use event::EventLoop;
```

- [ ] **Step 5: Verify workspace compiles with stub source files**

Create empty `app.rs`, `ui.rs`, `event.rs` so `lib.rs` module declarations compile.

```bash
cargo build -p agent007-tui 2>&1 | head -20
```

Expected: compiles (empty module files are acceptable at this step).

- [ ] **Step 6: Commit**

```bash
git add crates/tui/ Cargo.toml
git commit -m "feat(tui): bootstrap tui crate with TuiError skeleton"
```

---

### Task 2: App state

**Files:**
- Modify: `crates/tui/src/app.rs`

TUI layout from spec:

```
╔══════════════════════════════════════════════════════╗
║  agent007  v0.1.0          [q]uit  [p]ause  [?]help  ║
╠══════════════╦═══════════════════════════════════════╣
║ AGENTS       ║  TASK QUEUE                           ║
╠══════════════╬═══════════════════════════════════════╣
║ MODEL        ║  LOGS                                 ║
╠══════════════╬═══════════════════════════════════════╣
║ LEARNING     ║  RECENT OPTIMIZATIONS                 ║
╚══════════════╩═══════════════════════════════════════╝
```

- [ ] **Step 1: Write failing tests**

```rust
// In crates/tui/src/app.rs — test module only, paste at bottom after implementation
#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::types::{AgentId, Task, TaskResult};
    use agent007_core::events::AgentEvent;
    use uuid::Uuid;

    #[test]
    fn handle_task_assigned_adds_agent_to_list() {
        // construct App::default()
        // build AgentEvent::TaskAssigned { agent_id, task }
        // call app.handle_event(event)
        // assert agents list contains an AgentStatus for the given agent_id
        // assert status is Active or equivalent running state
    }

    #[test]
    fn handle_task_completed_marks_task_done() {
        // construct App::default()
        // send TaskAssigned, then TaskCompleted for the same agent_id
        // assert the task in app.tasks is marked as completed (success = true)
    }

    #[test]
    fn handle_learning_event_updates_learning_panel() {
        // construct App::default()
        // build LearningEvent::FeedbackRecorded { agent_id, reward: 0.8 }
        // call app.handle_learning_event(event)
        // assert app.learning_entries == 1
        // assert app.avg_reward is approximately 0.8
    }

    #[test]
    fn handle_learning_event_prompt_improved_appends_optimization() {
        // construct App::default()
        // build LearningEvent::PromptImproved { skill_name: "review-pr", old_reward: 0.2, new_reward: 0.7 }
        // call app.handle_learning_event(event)
        // assert app.recent_optimizations contains one entry
        // assert entry.skill_name == "review-pr"
    }

    #[test]
    fn paused_flag_toggles() {
        // construct App::default()
        // assert !app.paused
        // call app.toggle_pause()
        // assert app.paused
        // call app.toggle_pause()
        // assert !app.paused
    }
}
```

- [ ] **Step 2: Run to confirm tests fail**

```bash
cargo test -p agent007-tui app::tests 2>&1 | head -20
```

Expected: compile error — App not yet defined.

- [ ] **Step 3: Implement App state structs and methods (pseudocode)**

```rust
// crates/tui/src/app.rs — struct and method signatures, no bodies

use std::collections::VecDeque;
use agent007_core::types::AgentId;
use agent007_core::events::AgentEvent;
use agent007_learning::events::LearningEvent;

pub struct AgentStatus {
    pub agent_id: AgentId,
    pub name: String,
    pub state: AgentState,      // Active | Idle | Complete
}

pub enum AgentState { Active, Idle, Complete }

pub struct TaskStatus {
    pub task_id: uuid::Uuid,
    pub description: String,
    pub assigned_to: Option<AgentId>,
    pub done: bool,
    pub success: bool,
}

pub struct OptimizationSummary {
    pub skill_name: String,
    pub old_reward: f32,
    pub new_reward: f32,
}

pub struct ModelUsage {
    pub provider: String,       // "claude" | "codex" | "ollama/<model>"
    pub token_count: usize,
}

pub struct App {
    pub agents: Vec<AgentStatus>,
    pub tasks: VecDeque<TaskStatus>,
    pub logs: VecDeque<String>,
    pub model_usage: Vec<ModelUsage>,
    pub learning_entries: u32,
    pub avg_reward: f32,
    pub recent_optimizations: Vec<OptimizationSummary>,
    pub paused: bool,
    pub should_quit: bool,
    // Internal: log capacity cap
    log_capacity: usize,
}

impl App {
    pub fn new() -> Self;
    pub fn handle_event(&mut self, event: AgentEvent);
    pub fn handle_learning_event(&mut self, event: LearningEvent);
    /// Dispatch a keyboard action to the appropriate App method.
    pub fn handle_action(&mut self, action: crate::event::AppAction);
    pub fn toggle_pause(&mut self);
    pub fn quit(&mut self);
    fn push_log(&mut self, msg: String);  // trims to log_capacity
}

impl Default for App { fn default() -> Self; }
```

Key behavioural rules:
- `handle_event(AgentEvent::TaskAssigned { agent_id, task })` — upserts `AgentStatus` with `AgentState::Active`; pushes `TaskStatus` to `tasks`.
- `handle_event(AgentEvent::TaskCompleted { agent_id, result })` — marks matching task `done = true`, `success = result.success`; sets agent state to `Idle`.
- `handle_event(AgentEvent::ModelRequest { provider, token_estimate, .. })` — upserts or increments `ModelUsage` entry for `provider`.
- `handle_learning_event(LearningEvent::FeedbackRecorded { reward, .. })` — increments `learning_entries`; updates `avg_reward` as running mean.
- `handle_learning_event(LearningEvent::PromptImproved { skill_name, old_reward, new_reward })` — prepends to `recent_optimizations` (cap at 10 entries).
- `push_log` caps `logs` at `log_capacity` (default 200) by popping front when full.

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-tui app::tests
```

Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/tui/src/app.rs
git commit -m "feat(tui): implement App state with AgentEvent and LearningEvent handlers"
```

---

### Task 3: UI rendering

**Files:**
- Modify: `crates/tui/src/ui.rs`

- [ ] **Step 1: Write failing smoke test**

```rust
// In crates/tui/src/ui.rs — test module only

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::app::App;

    #[test]
    fn render_with_populated_app_does_not_panic() {
        // create TestBackend with width=120, height=40
        // create Terminal from TestBackend
        // construct App::default()
        // push two agents, two tasks, a few log lines, one OptimizationSummary
        // call terminal.draw(|f| render(f, &app))
        // assert result is Ok (smoke test — no panic)
    }

    #[test]
    fn render_empty_app_does_not_panic() {
        // create TestBackend, Terminal, App::default() (no data)
        // call terminal.draw(|f| render(f, &app))
        // assert result is Ok
    }
}
```

- [ ] **Step 2: Run to confirm tests fail**

```bash
cargo test -p agent007-tui ui::tests 2>&1 | head -20
```

Expected: compile error — `render` not defined.

- [ ] **Step 3: Implement render function (pseudocode)**

```rust
// crates/tui/src/ui.rs — signatures and layout structure, no widget detail bodies

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph};
use ratatui::style::{Color, Modifier, Style};
use crate::app::App;

/// Entry point called from EventLoop's terminal.draw() closure.
pub fn render(frame: &mut Frame, app: &App);

// Private helpers — each draws one panel into its Rect:
fn render_header(frame: &mut Frame, area: Rect, app: &App);
fn render_agents(frame: &mut Frame, area: Rect, app: &App);
fn render_task_queue(frame: &mut Frame, area: Rect, app: &App);
fn render_model(frame: &mut Frame, area: Rect, app: &App);
fn render_logs(frame: &mut Frame, area: Rect, app: &App);
fn render_learning(frame: &mut Frame, area: Rect, app: &App);
fn render_optimizations(frame: &mut Frame, area: Rect, app: &App);
```

Layout split (inside `render`):
1. Vertical split: `[header_row (3 lines), body_rows (remaining)]`.
2. Body split vertically into 3 equal-height rows.
3. Each body row split horizontally: left 30% | right 70%.
4. Row 1 left → `render_agents`; right → `render_task_queue`.
5. Row 2 left → `render_model`; right → `render_logs`.
6. Row 3 left → `render_learning`; right → `render_optimizations`.

Widgets used:
- `render_agents` → `List` of `ListItem`, bullet `●` for Active, `○` for Idle.
- `render_task_queue` → `List`, prefix `[✓]` done, `[→]` in-progress, `[ ]` queued.
- `render_model` → one `Gauge` per model provider (ratio = token_count / max_seen).
- `render_logs` → `Paragraph` with the last N lines of `app.logs`; auto-scrolls to bottom.
- `render_learning` → `Paragraph` with `Entries: N`, avg reward as text + `Gauge`.
- `render_optimizations` → `List` of recent `OptimizationSummary` lines.
- All panels wrapped in `Block::default().borders(Borders::ALL).title(...)`.

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-tui ui::tests
```

Expected: 2 tests pass (smoke — no panics).

- [ ] **Step 5: Commit**

```bash
git add crates/tui/src/ui.rs
git commit -m "feat(tui): implement render() with 6-panel ratatui layout"
```

---

### Task 4: EventLoop

**Files:**
- Modify: `crates/tui/src/event.rs`

- [ ] **Step 1: Write failing tests**

```rust
// In crates/tui/src/event.rs — test module only

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

    #[tokio::test]
    async fn quit_action_sets_should_quit() {
        // construct App::default()
        // call app.handle_action(AppAction::Quit)
        // assert app.should_quit == true
    }

    #[tokio::test]
    async fn pause_action_toggles_paused() {
        // construct App::default()
        // call app.handle_action(AppAction::Pause)
        // assert app.paused == true
        // call app.handle_action(AppAction::Pause)
        // assert app.paused == false
    }

    #[test]
    fn key_event_q_maps_to_quit_action() {
        // use crossterm::event::KeyCode::Char('q')
        // call map_key_event(KeyCode::Char('q'))
        // assert result == Some(AppAction::Quit)
    }

    #[test]
    fn key_event_p_maps_to_pause_action() {
        // call map_key_event(KeyCode::Char('p'))
        // assert result == Some(AppAction::Pause)
    }

    #[test]
    fn unknown_key_maps_to_none() {
        // call map_key_event(KeyCode::Char('z'))
        // assert result == None
    }
}
```

- [ ] **Step 2: Run to confirm tests fail**

```bash
cargo test -p agent007-tui event::tests 2>&1 | head -20
```

Expected: compile error — EventLoop, AppAction not defined.

- [ ] **Step 3: Implement EventLoop (pseudocode)**

```rust
// crates/tui/src/event.rs — signatures only, no bodies

use crossterm::event::KeyCode;
use agent007_core::events::AgentEvent;
use agent007_learning::events::LearningEvent;
use crate::app::App;
use crate::TuiError;

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    Quit,
    Pause,
    Help,
}

/// Maps a crossterm KeyCode to an AppAction (or None for unbound keys).
pub fn map_key_event(key: KeyCode) -> Option<AppAction>;

pub struct EventLoop {
    /// Receives AgentEvents from the core Dispatcher stream.
    agent_event_rx: tokio::sync::mpsc::Receiver<AgentEvent>,
    /// Receives LearningEvents from the LearningDispatcher stream.
    learning_event_rx: tokio::sync::mpsc::Receiver<LearningEvent>,
}

impl EventLoop {
    /// Construct by subscribing to both dispatchers.
    /// Spawns two background tasks that forward events into mpsc channels.
    pub async fn new(
        dispatcher: std::sync::Arc<dyn agent007_core::dispatcher::Dispatcher>,
        learning_dispatcher: std::sync::Arc<agent007_learning::LearningDispatcher>,
    ) -> Result<Self, TuiError>;

    /// Run the terminal event loop until app.should_quit is true.
    ///
    /// - Enters raw mode, creates alternate screen, initialises Terminal<CrosstermBackend>.
    /// - tokio::select! on: crossterm EventStream, agent_event_rx, learning_event_rx.
    /// - Keyboard event → map_key_event → app.handle_action.
    /// - AgentEvent → app.handle_event (if !app.paused).
    /// - LearningEvent → app.handle_learning_event (if !app.paused).
    /// - After each iteration: terminal.draw(|f| render(f, &app)).
    /// - On exit: leave alternate screen, disable raw mode (runs even on error via scopeguard).
    pub async fn run(
        mut self,
        app: &mut App,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), TuiError>;
}
```

Shutdown contract:
- When `app.should_quit` is true (user pressed `q`), `run` cancels the `CancellationToken` before returning, triggering graceful shutdown of the `Orchestrator` and all worker agents via `TaskTracker::wait()`.
- If the `CancellationToken` is cancelled externally (SIGINT/SIGTERM), the `select!` loop detects it via `cancel.cancelled()` and exits cleanly.

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-tui event::tests
```

Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/tui/src/event.rs
git commit -m "feat(tui): implement EventLoop with keyboard + event stream fan-in and graceful shutdown"
```

---

## Chunk 2: CLI wiring

The `cli` crate is a stub from Plan 1. This chunk completes it to wire all Phase 1 crates together so `agent007 run "<task>"` works end-to-end.

### File Structure (Chunk 2)

```
crates/cli/src/
├── main.rs             # tokio runtime entry, arg parsing dispatch
├── config.rs           # Config struct + Config::load()
└── commands/
    ├── mod.rs
    ├── run.rs          # `agent007 run "<task>"` — full stack + TUI
    ├── skill.rs        # `agent007 skill add/list/run`
    └── simulate.rs     # `agent007 simulate <template>` — Phase 2 stub
```

---

### Task 5: Config loading

**Files:**
- Create: `crates/cli/src/config.rs`
- Modify: `crates/cli/Cargo.toml` (add toml, serde deps)
- Modify: `crates/cli/src/main.rs` (add `mod config`)

- [ ] **Step 1: Write failing tests**

```rust
// In crates/cli/src/config.rs — test module only

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CONFIG: &str = r#"
[core]
max_agents = 4
task_queue_capacity = 128

[models]
default = "claude"

[models.routing]
code_completion = "codex"
reasoning = "claude"
fast_local = "ollama"
sensitive = "ollama"

[models.ollama]
base_url = "http://localhost:11434"
default_model = "llama3"

[memory.rag]
enabled = true
vector_db = "lancedb"
embedding_provider = "ollama"
embedding_model = "nomic-embed-text"
index = ["./src", "./docs"]

[mcp.servers]
filesystem = "npx @modelcontextprotocol/server-filesystem"

[ide]
port = 7007

[learning]
enabled = true
optimizer_threshold = 0.3
optimizer_trigger_count = 10
optimizer_model = "claude"

[learning.reward_weights]
completion = 0.4
user_rating = 0.3
tool_errors = 0.2
retries = 0.1
"#;

    #[test]
    fn parse_full_config_toml() {
        // parse SAMPLE_CONFIG via Config::from_str (or toml::from_str)
        // assert config.core.max_agents == 4
        // assert config.core.task_queue_capacity == 128
        // assert config.models.default == "claude"
        // assert config.models.routing.code_completion == Some("codex")
        // assert config.models.ollama.base_url == "http://localhost:11434"
        // assert config.memory.rag.enabled == true
        // assert config.memory.rag.vector_db == "lancedb"
        // assert config.ide.port == 7007
        // assert config.learning.enabled == true
        // assert config.learning.reward_weights.completion == 0.4
    }

    #[test]
    fn config_defaults_are_sensible() {
        // parse a minimal config (just [core]) via Config::from_str
        // assert config.models.default == "claude"  (default value)
        // assert config.ide.port == 7007             (default value)
        // assert config.learning.enabled == false    (safe default: off)
    }

    #[test]
    fn config_load_respects_agent007_config_env() {
        // write SAMPLE_CONFIG to a tempfile
        // set env AGENT007_CONFIG = tempfile path
        // call Config::load()
        // assert it reads the tempfile path (not ~/.agent007/config.toml)
        // unset AGENT007_CONFIG after test
    }
}
```

- [ ] **Step 2: Run to confirm tests fail**

```bash
cargo test -p agent007 config::tests 2>&1 | head -20
```

Expected: compile error — Config not defined.

- [ ] **Step 3: Update cli Cargo.toml with new deps**

```toml
# crates/cli/Cargo.toml — additions to [dependencies]:
toml = { workspace = true }
serde = { workspace = true }
clap = { version = "4", features = ["derive"] }

# Add clap to workspace deps too:
# In root Cargo.toml [workspace.dependencies]:
# clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 4: Implement Config structs and Config::load (pseudocode)**

```rust
// crates/cli/src/config.rs — struct and method signatures, no bodies

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use anyhow::Result;

#[derive(Debug, Deserialize, Serialize)]
pub struct CoreConfig {
    pub max_agents: usize,          // default: 8
    pub task_queue_capacity: usize, // default: 256
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RoutingConfig {
    pub code_completion: Option<String>,
    pub reasoning: Option<String>,
    pub fast_local: Option<String>,
    pub sensitive: Option<String>,
    pub default: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OllamaModelConfig {
    pub base_url: String,           // default: "http://localhost:11434"
    pub default_model: String,      // default: "llama3"
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ModelsConfig {
    pub default: String,            // default: "claude"
    pub routing: Option<RoutingConfig>,
    pub ollama: Option<OllamaModelConfig>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RagConfig {
    pub enabled: bool,
    pub vector_db: String,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub index: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MemoryConfig {
    pub rag: Option<RagConfig>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct McpConfig {
    pub servers: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct IdeConfig {
    pub port: u16,  // default: 7007
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RewardWeightsConfig {
    pub completion: f32,    // default: 0.4
    pub user_rating: f32,   // default: 0.3
    pub tool_errors: f32,   // default: 0.2
    pub retries: f32,       // default: 0.1
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LearningConfig {
    pub enabled: bool,                      // default: false
    pub optimizer_threshold: f32,           // default: 0.3
    pub optimizer_trigger_count: usize,     // default: 10
    pub optimizer_model: String,            // default: "claude"
    pub reward_weights: RewardWeightsConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub core: CoreConfig,
    pub models: ModelsConfig,
    pub memory: Option<MemoryConfig>,
    pub mcp: Option<McpConfig>,
    pub ide: IdeConfig,
    pub learning: LearningConfig,
}

impl Config {
    /// Parse from a TOML string (for tests and internal use).
    pub fn from_str(s: &str) -> Result<Self>;

    /// Load config from AGENT007_CONFIG env var path, or ~/.agent007/config.toml.
    /// If neither exists, returns Config::default() (all defaults, no error).
    pub fn load() -> Result<Self>;

    /// Default config path: ~/.agent007/config.toml
    fn default_path() -> PathBuf;
}

impl Default for Config { fn default() -> Self; }
```

`serde` field defaults are set via `#[serde(default = "...")]` attributes so missing TOML fields fall back to sensible values rather than causing parse errors.

- [ ] **Step 5: Run tests**

```bash
cargo test -p agent007 config::tests
```

Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/config.rs crates/cli/Cargo.toml Cargo.toml
git commit -m "feat(cli): add Config struct with full Phase 1 TOML schema and Config::load()"
```

---

### Task 6: CLI arg parsing

**Files:**
- Modify: `crates/cli/src/main.rs`
- Create: `crates/cli/src/commands/mod.rs`
- Create: `crates/cli/src/commands/run.rs` (stub)
- Create: `crates/cli/src/commands/skill.rs` (stub)
- Create: `crates/cli/src/commands/simulate.rs` (stub)

- [ ] **Step 1: Write failing test**

```rust
// In crates/cli/src/main.rs — test module only

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_run_subcommand() {
        // Cli::try_parse_from(["agent007", "run", "say hello"])
        // assert matches!(cli.command, Commands::Run { task } if task == "say hello")
    }

    #[test]
    fn parse_skill_list_subcommand() {
        // Cli::try_parse_from(["agent007", "skill", "list"])
        // assert matches!(cli.command, Commands::Skill(SkillArgs::List))
    }

    #[test]
    fn parse_skill_add_subcommand() {
        // Cli::try_parse_from(["agent007", "skill", "add", "/path/to/skill.md"])
        // assert matches!(cli.command, Commands::Skill(SkillArgs::Add { path }) if path == "/path/to/skill.md")
    }

    #[test]
    fn parse_slash_trigger_as_skill_run() {
        // Cli::try_parse_from(["agent007", "/review-pr", "https://github.com/org/repo/pull/42"])
        // assert matches!(cli.command, Commands::Skill(SkillArgs::Run { trigger, args })
        //   if trigger == "/review-pr" && args == "https://github.com/org/repo/pull/42")
    }
}
```

- [ ] **Step 2: Run to confirm tests fail**

```bash
cargo test -p agent007 tests 2>&1 | head -20
```

Expected: compile error — Cli, Commands not defined.

- [ ] **Step 3: Implement CLI arg structs (pseudocode)**

```rust
// crates/cli/src/main.rs — struct definitions, no command handler bodies

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "agent007", version = "0.1.0", about = "Multi-agent AI orchestration")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a task with the full agent stack
    Run {
        /// The task description to execute
        task: String,
    },
    /// Manage skills
    Skill(SkillArgs),
    /// Run a simulation template (Phase 2 stub)
    Simulate {
        /// Template name
        template: String,
    },
    /// Slash-command trigger (e.g. /review-pr <args>)
    #[command(external_subcommand)]
    Slash(Vec<String>),
}

#[derive(Parser, Debug)]
pub struct SkillArgs {
    #[command(subcommand)]
    pub action: SkillAction,
}

#[derive(Subcommand, Debug)]
pub enum SkillAction {
    /// List loaded skills
    List,
    /// Add a skill file to ~/.agent007/skills/
    Add {
        /// Path to the skill markdown file
        path: String,
    },
    /// Run a skill by trigger
    Run {
        /// Skill trigger (e.g. /review-pr)
        trigger: String,
        /// Arguments passed to the skill template
        args: String,
    },
}
```

The `main` function matches on `Commands` and dispatches to the relevant handler in `commands/`:

```rust
// crates/cli/src/main.rs — main signature only

#[tokio::main]
async fn main() -> anyhow::Result<()>;
// - parse Cli
// - Config::load() → Arc<Config>
// - match cli.command { Run { task } => commands::run::execute(config, task).await, ... }
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007 tests
```

Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/main.rs crates/cli/src/commands/
git commit -m "feat(cli): add clap subcommand structure — run/skill/simulate/slash"
```

---

### Task 7: `run` command — full stack wiring

**Files:**
- Modify: `crates/cli/src/commands/run.rs`
- Modify: `crates/cli/Cargo.toml` (add all Phase 1 crate deps)

- [ ] **Step 1: Update cli Cargo.toml with all Phase 1 crate deps**

```toml
# crates/cli/Cargo.toml [dependencies] — add:
agent007-core     = { path = "../core" }
agent007-models   = { path = "../models" }
agent007-memory   = { path = "../memory" }
agent007-skills   = { path = "../skills" }
agent007-hooks    = { path = "../hooks" }
agent007-mcp      = { path = "../mcp" }
agent007-learning = { path = "../learning" }
agent007-tui      = { path = "../tui" }
tokio-util        = { workspace = true }
tracing-subscriber = { workspace = true }
```

- [ ] **Step 2: Write failing integration test**

```rust
// crates/cli/src/commands/run.rs — test module only

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::config::Config;

    #[tokio::test]
    async fn run_command_builds_stack_without_panic() {
        // set env AGENT007_DRY_RUN=1 (MockProvider path)
        // construct a minimal Config::default()
        // call build_stack(&config) — returns all constructed subsystems
        // assert all returned Arc handles are Some/Ok
        // do NOT call execute_task — just verify construction succeeds
    }
}
```

- [ ] **Step 3: Run to confirm test fails**

```bash
cargo test -p agent007 commands::run::tests 2>&1 | head -20
```

Expected: compile error.

- [ ] **Step 4: Implement run command (pseudocode)**

```rust
// crates/cli/src/commands/run.rs — function signatures, no bodies

use std::sync::Arc;
use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::config::Config;
use agent007_core::dispatcher::LocalDispatcher;
use agent007_core::orchestrator::Orchestrator;
use agent007_models::router::ModelRouter;
use agent007_memory::{MemoryStore, Retriever, Indexer};
use agent007_skills::{SkillLoader, SkillExecutor};
use agent007_hooks::HookExecutor;
use agent007_mcp::McpClient;
use agent007_learning::{FeedbackCollector, LearningDispatcher};
use agent007_tui::{App, EventLoop};

pub struct Stack {
    pub dispatcher: Arc<LocalDispatcher>,
    pub orchestrator: Arc<Orchestrator>,
    pub model_router: Arc<ModelRouter>,
    pub memory_store: Arc<MemoryStore>,
    pub retriever: Arc<Retriever>,
    pub indexer: Arc<Indexer>,
    pub skill_loader: Arc<SkillLoader>,
    pub skill_executor: Arc<SkillExecutor>,
    pub hook_executor: Arc<HookExecutor>,
    pub mcp_client: Arc<McpClient>,
    pub feedback_collector: Arc<FeedbackCollector>,
    pub learning_dispatcher: Arc<LearningDispatcher>,
    pub cancel: CancellationToken,
    pub tracker: TaskTracker,
}

/// Construct all Phase 1 subsystems from config.
/// When AGENT007_DRY_RUN=1, injects MockProvider instead of real providers.
pub async fn build_stack(config: &Config) -> Result<Stack>;

/// Top-level entry point for `agent007 run "<task>"`.
pub async fn execute(config: Arc<Config>, task: String) -> Result<()>;
// Implementation order:
// 1. build_stack(&config)
// 2. stack.indexer.index_paths(&config.memory.rag.index) — background task
// 3. stack.orchestrator.submit_task(task) — enqueue user task
// 4. construct App::default()
// 5. EventLoop::new(stack.dispatcher, stack.learning_dispatcher).await
// 6. eventloop.run(&mut app, stack.cancel.clone()).await
//    — this blocks until user presses q or task completes
// 7. stack.tracker.wait().await  — graceful shutdown: all agents drain
```

Construction order inside `build_stack`:
1. `CancellationToken::new()` and `TaskTracker::new()`.
2. `ModelRouter::from_config(config)` — picks `MockProvider` if `AGENT007_DRY_RUN=1`.
3. `LocalDispatcher::new(capacity: 1024)`.
4. `MemoryStore::new(~/.agent007/memory)`.
5. `Indexer::new(embedding_provider, lancedb_store)`.
6. `Retriever::new(embedding_provider, lancedb_store)`.
7. `SkillLoader::from_dir(~/.agent007/skills)`.
8. `SkillExecutor::new(skill_loader, model_router, retriever)`.
9. `HookExecutor::from_file(~/.agent007/hooks/hooks.toml)`.
10. `McpClient::from_config(mcp_servers)`.
11. `LearningDispatcher::new(capacity: 512)`.
12. `FeedbackCollector::new(dispatcher.subscribe(), learning_dispatcher)`.
13. `Orchestrator::new(dispatcher, model_router, memory_store, persona_provider: Arc::new(NoOpPersonaProvider), cancel, tracker)`.

- [ ] **Step 5: Run test**

```bash
cargo test -p agent007 commands::run::tests
```

Expected: 1 test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/commands/run.rs crates/cli/Cargo.toml
git commit -m "feat(cli): implement run command — wires all Phase 1 crates into Stack, starts TUI"
```

---

### Task 8: `skill` command

**Files:**
- Modify: `crates/cli/src/commands/skill.rs`

- [ ] **Step 1: Write failing tests**

```rust
// In crates/cli/src/commands/skill.rs — test module only

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn skill_add_copies_file_to_skills_dir() {
        // create a TempDir as the skills dir
        // create a sample skill .md file in another TempDir
        // call copy_skill_to_dir(skill_file_path, skills_dir)
        // assert the file exists at skills_dir/skill_name.md
    }

    #[tokio::test]
    async fn skill_list_prints_loaded_skills() {
        // create a TempDir with two skill .md files (valid frontmatter)
        // call list_skills(skills_dir)
        // assert returns a Vec<SkillSummary> with 2 entries
    }

    #[tokio::test]
    async fn skill_run_calls_executor_with_correct_trigger() {
        // construct a MockSkillExecutor (or use real SkillExecutor with MockProvider)
        // call run_skill("/review-pr", "https://...", &executor)
        // assert executor was called with trigger "/review-pr"
    }
}
```

- [ ] **Step 2: Run to confirm tests fail**

```bash
cargo test -p agent007 commands::skill::tests 2>&1 | head -20
```

Expected: compile error.

- [ ] **Step 3: Implement skill command (pseudocode)**

```rust
// crates/cli/src/commands/skill.rs — signatures only

use std::path::{Path, PathBuf};
use anyhow::Result;
use agent007_skills::{SkillExecutor, SkillSummary};

/// Copy a skill file into the skills directory. File is renamed to its `name` field.
pub fn copy_skill_to_dir(skill_path: &Path, skills_dir: &Path) -> Result<()>;

/// List all skills found in skills_dir. Returns a Vec of summaries (name + description).
pub async fn list_skills(skills_dir: &Path) -> Result<Vec<SkillSummary>>;

/// Execute a skill by trigger string with provided args string.
pub async fn run_skill(
    trigger: &str,
    args: &str,
    executor: &SkillExecutor,
) -> Result<String>;

/// Top-level dispatch for `agent007 skill <action>`.
pub async fn execute(config: std::sync::Arc<crate::config::Config>, action: crate::SkillAction) -> Result<()>;
// - SkillAction::List → list_skills(~/.agent007/skills), print table
// - SkillAction::Add { path } → copy_skill_to_dir(path, ~/.agent007/skills/)
// - SkillAction::Run { trigger, args } → build minimal stack (model_router + skill_executor), run_skill
```

`agent007 /review-pr <args>` (slash trigger from `Commands::Slash`) is mapped to `SkillAction::Run` in `main.rs` before dispatching to `skill::execute`.

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007 commands::skill::tests
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/commands/skill.rs
git commit -m "feat(cli): implement skill command — list/add/run with slash-trigger support"
```

---

### Task 9: `simulate` stub + end-to-end smoke test

**Files:**
- Modify: `crates/cli/src/commands/simulate.rs`

- [ ] **Step 1: Write failing end-to-end smoke test**

```rust
// crates/cli/tests/e2e_smoke.rs

#[tokio::test]
async fn run_say_hello_with_dry_run_env() {
    // set AGENT007_DRY_RUN=1 (MockProvider, no real API calls)
    // construct Config::default()
    // call commands::run::execute(Arc::new(config), "say hello".to_string())
    // assert result is Ok
    // This test verifies: stack builds, Orchestrator receives task, MockProvider
    // returns response, FeedbackCollector records it, TUI renders at least one frame
    // and exits cleanly when the task completes (Orchestrator signals cancel token).
    // unset AGENT007_DRY_RUN after test
}
```

- [ ] **Step 2: Run to confirm test fails**

```bash
cargo test -p agent007 --test e2e_smoke 2>&1 | head -20
```

Expected: compile error or test failure (stack not complete yet).

- [ ] **Step 3: Add simulate stub**

```rust
// crates/cli/src/commands/simulate.rs

use anyhow::Result;
use std::sync::Arc;
use crate::config::Config;

/// Phase 2 stub — prints a "not yet implemented" message.
pub async fn execute(config: Arc<Config>, template: String) -> Result<()>;
```

- [ ] **Step 4: Run full test suite**

```bash
cargo test -p agent007 2>&1
```

Expected: all tests in `cli` pass including e2e smoke test.

- [ ] **Step 5: Run workspace-level check**

```bash
cargo build --workspace 2>&1 | head -30
```

Expected: all crates build cleanly. Zero errors.

- [ ] **Step 6: Commit + tag Phase 1 complete**

```bash
git add crates/cli/src/commands/simulate.rs crates/cli/tests/
git commit -m "feat(cli): add simulate stub; end-to-end smoke test passes — Phase 1 complete"
git tag -a v0.1.0 -m "Phase 1 complete: agent007 run end-to-end with TUI"
```

---

## Success Criteria (Phase 1 Complete)

After executing all tasks in this plan:

- [ ] `cargo build --workspace` succeeds with zero errors.
- [ ] `cargo test --workspace` — all tests pass; zero real API calls (MockProvider gate).
- [ ] `agent007 run "say hello"` with `AGENT007_DRY_RUN=1` renders the TUI and completes the task without panicking.
- [ ] TUI shows all 6 panels: AGENTS, TASK QUEUE, MODEL, LOGS, LEARNING, RECENT OPTIMIZATIONS.
- [ ] Pressing `q` in the TUI triggers graceful shutdown: `CancellationToken` cancelled, `TaskTracker::wait()` completes, process exits cleanly.
- [ ] `agent007 skill list` prints loaded skills from `~/.agent007/skills/`.
- [ ] `agent007 skill add <path>` copies the skill file to `~/.agent007/skills/`.
- [ ] `agent007 /review-pr <args>` maps to `SkillExecutor::execute` with the correct trigger.
- [ ] `agent007 simulate <template>` prints "not yet implemented" (Phase 2 stub).
- [ ] All library crates (`tui`) use `thiserror`. Only `cli` uses `anyhow`.
- [ ] No raw prompt text or API keys flow through the event bus (all `PromptRef` / `MemoryRef` opaque handles).
