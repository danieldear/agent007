# Architecture

agent007 is a layered Rust workspace of 18 crates. The CLI binary (`crates/cli`) is the only binary; all other crates are libraries.

## Crate map

```
crates/
├── cli/          Binary entry point — clap CLI + MCP server (serve.rs) + web dashboard
├── core/         Foundational types: Task, AgentId, Dispatcher, Orchestrator, budget, context
├── models/       LLM provider abstraction (Claude, Codex, Ollama) + ModelRouter + Adaptive Shadow
├── memory/       Scoped key-value store + LanceDB vector search + MemoryStore API
├── skills/       Skill loading, frontmatter parsing (SkillFrontmatter), SkillDispatcher
├── hooks/        HookConfig (hooks.toml) + HookExecutor (spawns shell commands)
├── learning/     LearningStore + FeedbackEntry recording + PromptOptimizer + InsightGenerator
├── personas/     PersonaSpec loading from ~/.agent007/personas/*.md
├── workflows/    WorkflowDef YAML schema + step graph + WorkflowEngine + hosted engine + eval gates + reliability engine
├── mcp/          MCP client (connects to downstream MCP servers) + tool proxy
├── tui/          Ratatui-based terminal dashboard
├── web/          Axum web dashboard (WebSocket + REST) served on --port
├── ide-bridge/   LSP server (tower-lsp) — `agent007 serve-lsp`
├── zones/        File-path access control (zone rules)
├── git-agent/    git2-based branch/commit/PR helpers
├── custom-agents/ Custom agent YAML loader
├── testing/      AI testing pipeline helpers
└── simulation/   Simulation template runner
```

## Data flow

```
AI Editor (Claude Code / Cursor / Codex / Copilot / Zed)
        │
        │  MCP (stdio)
        ▼
┌──────────────────────────────────────────────────────────┐
│  crates/cli — Agent007Server (serve.rs)                  │
│                                                          │
│  call_tool() dispatcher                                  │
│    ├── agent007_run ──────────────► core::Orchestrator   │
│    │                                    │                │
│    │                               models::ModelRouter   │
│    │                                    │                │
│    │                          skills::SkillDispatcher    │
│    │                                    │                │
│    │                            LLM API (Claude/Codex)   │
│    │                                                     │
│    ├── agent007_skill_run ─────► skills::SkillDispatcher │
│    │       └── fire_hook(OnSkillExecute)                 │
│    │                                                     │
│    ├── agent007_memory_write ──► memory::MemoryStore     │
│    │       └── fire_hook(OnMemoryWrite)                  │
│    │                                                     │
│    ├── agent007_record_tokens ─► learning::LearningStore │
│    │       │  └── fire_hook(PostTaskComplete)           │
│    │       └──────────────────► InsightGenerator        │
│    │                            (writes project memory  │
│    │                             when failure patterns  │
│    │                             are detected)          │
│    │                                                     │
│    ├── agent007_workflow_* ────► workflows::WorkflowEngine│
│    │       ├── eval gates (pass/warn/block per run)    │
│    │       ├── reliability engine (budget, guardrails, │
│    │       │   confidence escalation, retry transitions)│
│    │       └── hosted engine (session-based step loop) │
│    │                                                     │
│    └── agent007_mcp_tool_call ─► mcp::McpClient          │
│                                   (downstream MCP servers)│
└──────────────────────────────────────────────────────────┘
        │
        │  HTTP + WebSocket
        ▼
┌──────────────────────┐
│  web dashboard :8007 │
│  (crates/web)        │
└──────────────────────┘
```

## Key interfaces

### `core::Dispatcher`
```rust
pub trait Dispatcher: Send + Sync {
    async fn dispatch(&self, task: Task) -> Result<TaskResult>;
}
```
`LocalDispatcher` implements this directly. The `learning_dispatcher` wraps it and records `FeedbackEntry` on each completion.

### `models::ModelProvider`
```rust
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse>;
}
```
`CompletionResponse` carries `input_tokens` and `output_tokens` parsed from the provider's API `usage` field.

### `skills::SkillDispatcher`
Loads skills from `~/.agent007/skills/` (global) and `.agent007/skills/` (project-local). Project skills override global skills with the same trigger. Each skill is a `SkillFrontmatter` + template body rendered with Tera.

### `memory::MemoryStore`
Scoped key-value with optional LanceDB backend. Scopes are namespaced subdirectories under `~/.agent007/memory/`. `MemoryStore::scoped("name")` requires `Arc<MemoryStore>`.

### `hooks::HookExecutor`
Reads `HookConfig` from `hooks.toml`. `fire(event, env_vars)` spawns a subprocess synchronously. Hook commands receive context via environment variables (`HOOK_SKILL`, `HOOK_KEY`, etc.).

### `learning::LearningStore`
`LearningStore::new(ScopedMemoryStore)` + `record_feedback(&FeedbackEntry)`. Each `FeedbackEntry` has: `id`, `agent_id`, `model`, `skill_name`, `outcome`, `reward`, `timestamp`. Written to the `learning` scope in memory. Additional methods: `count_feedback(skill)`, `list_skill_names()`.

### `learning::InsightGenerator`
Attached to `FeedbackCollector` via `with_insight_generator(Arc<InsightGenerator>)`. After each `TaskCompleted` event, checks whether the skill's feedback count crosses a multiple of `check_every_n`. If the failure rate exceeds `min_failure_rate`, calls the configured LLM model and writes a `type: procedural` memory entry to the `project` scope. The entry is immediately available via `{{memory.project}}` and `{{rag_context}}`. See ADR-007.

### `workflows::WorkflowEngine`
Parses `~/.agent007/workflows/<name>.yaml`. Builds a dependency graph (petgraph). Steps without `depends_on` run in parallel. Approval gates block progression until `workflow_approve` is called. Hosted-MCP mode: the engine emits step prompts; the host LLM executes and submits outputs back.

**V2 subsystems (opt-in, backward-compatible):**
- **Eval Gates** (`eval_gates.rs`) — score each run against a rolling baseline; make `pass / warn / block` decisions. Configurable per workflow via `eval_gate:` YAML block.
- **Reliability Engine** — four additive controls: budget governor (token spend tracking with graceful degradation), guardrails hook (pre-step safety check), confidence-driven escalation (routes low-confidence output to human approval), recovery transitions (bounded retry with explicit transition records).
- **Hosted Engine** (`hosted.rs`) — session-based loop for multi-step workflows where the host LLM runs each step. Approval ownership follows the initiating client.

### `models::ModelRouter` + Adaptive Shadow
Rule-based routing selects the model for each task. Adaptive Shadow runs alongside every step, recording which route *would* have been recommended based on historical performance — without changing the actual route. Shadow recommendations accumulate in the run record and surface in the web dashboard under **Routing Recommendations**.

## Directory layout (~/.agent007/)

```
~/.agent007/
├── config.toml          Main configuration
├── skills/              User-defined skills (*.md)
├── personas/            User-defined personas (*.md)
├── workflows/           User-defined workflows (*.yaml)
├── hooks/
│   └── hooks.toml       Hook configuration
├── memory/
│   ├── global/          Global key-value
│   ├── project/         Project-scoped key-value
│   ├── user/            User-scoped key-value
│   └── learning/        FeedbackEntry records
└── sessions/            Run history (JSON per run)
```
