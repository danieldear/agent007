# Multi-Agent Execution

agent007 ships a full multi-agent orchestration layer built around three
complementary concepts:

| Layer | Concept | Answers |
|-------|---------|---------|
| **Workflow** | Topology — who talks to whom, in what order | _When_ and _with whom_ |
| **Persona** | Behaviour — system prompt, model, zones, memory | _How_ the agent acts |
| **Skill** | Domain knowledge — injected Markdown bodies | _What_ the agent knows |

A **persona IS the agent**. No separate AgentDef TOML is needed.
Workflows orchestrate personas into teams via the `multi-agent` step type;
skills augment a persona's system prompt on demand.

---

## Concepts

| Term | Description |
|------|-------------|
| **PersonaSpec** | TOML file in `~/.agent007/personas/` (or built-in). Now also carries `skills`, `agent_type`, `allowed_workers`, `memory_namespace`, and `zones`. |
| **SkillContentProvider** | Trait that resolves a skill trigger name → Markdown body. `SkillIndex` (loaded from disk) and `NoOpSkillContentProvider` (fallback) implement it. |
| **SubOrchestrator** | Rust struct that owns the decompose → dispatch → replan → synthesise loop. Can be constructed from a `PersonaSpec` via `SubOrchestrator::from_persona()`. |
| **WorkerOutput** | Per-subtask result: output string + optional blocker reason |
| **SubTaskResult** | Final result returned to the caller: combined `output`, `blockers`, `files_changed`, `tests_passed` |
| **Dispatcher** | Broadcast channel; publishes `AgentEvent` variants consumed by the dashboard metrics engine and TUI |

---

## Persona TOML schema (extended)

Standard persona fields plus five new optional fields that enable agent behaviour:

```toml
# ~/.agent007/personas/feature-lead.toml
name             = "feature-lead"
description      = "Owns feature delivery end-to-end"
system_prompt    = "You are the feature lead. Break down tasks and coordinate specialists."
preferred_model  = "claude-opus-4"
allowed_tools    = ["bash", "file_read", "file_write"]

# ── Agent lifecycle (all optional) ──────────────────────────────────────────

# Scoped memory prefix (defaults to name at runtime)
memory_namespace = "feature-lead"

# Default skills always loaded into the system prompt
skills = ["dev-architect", "dev-debug"]

# "orchestrator" → can run as a SubOrchestrator
# "worker"       → leaf node (default)
agent_type = "orchestrator"

# Worker persona names this orchestrator may delegate to
allowed_workers = ["coder", "codereviewer", "testdesigner"]

# Optional zone rules
[zones]
readonly  = ["docs/"]
forbidden = [".env", "secrets/"]
```

---

## Workflow `multi-agent` step type

A workflow step of type `multi-agent` spins up a `SubOrchestrator` from the
named persona, injects its skill domain knowledge, and dispatches workers
defined in the step:

```toml
# ~/.agent007/workflows/feature-delivery.toml
name    = "feature-delivery"
version = "1.0.0"

[[steps]]
id    = "plan-and-implement"
type  = "multi-agent"
agent = "feature-lead"          # orchestrator persona name

[[steps.workers]]
persona = "coder"               # worker persona name
run     = "parallel"            # "parallel" (default) or "sequential"

[[steps.workers]]
persona = "codereviewer"
run     = "parallel"

[[steps.workers]]
persona = "testdesigner"
run     = "sequential"          # runs after parallel workers finish
```

### Worker run modes

| Mode | Behaviour |
|------|-----------|
| `parallel` (default) | All parallel workers run concurrently via `JoinSet` |
| `sequential` | Runs after all parallel workers; receives their combined output as context prefix |

---

## Skill injection

When `SubOrchestrator::from_persona()` is called, each trigger listed in
`persona.skills` is resolved through the `SkillContentProvider`. Found bodies
are prepended to the system prompt:

```
## Domain Knowledge

<skill body>

---

<original system prompt>
```

Multiple skills stack: each prepends to the already-injected prompt.
Unknown triggers are silently skipped (no-op).

### Explicit skill override in a workflow step

Workflow steps can also specify skills directly on individual workers:

```toml
[[steps.workers]]
persona = "coder"
skills  = ["code-optimize", "code-security-audit"]
```

These are merged with the persona's own `skills` list before injection.

---

---

## CLI usage

```
agent007 agent list                     List all registered agents
agent007 agent inspect <name>           Show agent definition details
agent007 agent run <name> "<task>"      Run an agent on a task (uses PersonaSpec)
agent007 workflow run <name> "<task>"   Run a full workflow (can include multi-agent steps)
```

### Run an agent directly from its persona

```bash
# Run any persona as an orchestrator (uses persona.allowed_workers)
agent007 agent run feature-lead "Add dark-mode toggle to the settings page"
```

### Run a workflow that includes a multi-agent step

```bash
agent007 workflow run feature-delivery "Add dark-mode toggle to the settings page"
```

Output:

```
🤖 Running workflow 'feature-delivery' …
   Step: plan-and-implement [multi-agent → feature-lead]

[coder]
Implemented ToggleDarkMode component with CSS variable swap …

[codereviewer]
No blocking issues. Suggested extracting the theme token list …

[testdesigner]
Added 3 unit tests for ToggleDarkMode and 1 integration test …
```

---

## MCP tool

When `agent007 serve` is running, the sub-orchestrator is also exposed as an MCP
tool so Claude Code (or any MCP client) can trigger multi-agent runs directly:

```json
{
  "tool": "agent007_agent_run",
  "arguments": {
    "name": "feature-dev",
    "task": "Add dark-mode toggle to the settings page"
  }
}
```

The result is returned as a text block containing:
- Combined worker outputs
- `Blockers:` section (if any worker was blocked after replanning)
- `Files changed:` section (if any worker reported modified files)

---

## Execution flow

```
SubOrchestrator.run(task)
  │
  ├─ 1. Depth guard (max 3 levels of nesting by default)
  ├─ 2. Empty-workers guard → return error if allowed_workers is empty
  │
  ├─ 3. plan(task) → Vec<SubTask>
  │      Model call:  system_prompt + "decompose into subtasks JSON"
  │      Returns:     [{worker_name, description}, …]
  │
  ├─ 4. dispatch_parallel(subtasks) → Vec<WorkerOutput>
  │      JoinSet spawns one async task per subtask.
  │      Each task:
  │        a. Publishes AgentEvent::ModelRequest
  │        b. Looks up persona for worker_name → gets system_prompt
  │        c. Calls ModelRouter::complete()
  │        d. On success → AgentEvent::WorkerResult, blocker = None
  │        e. On failure → AgentEvent::WorkerBlocked, blocker = Some(reason)
  │
  ├─ 5. drain_blocked() → split into (completed, blocked)
  │
  ├─ 6. Dynamic replan (if any blocked):
  │        Second model call describing blocked subtasks only
  │        → revised Vec<SubTask> → re-dispatch
  │        (replan happens at most once per run)
  │
  ├─ 7. persist_synthesis()
  │        Writes JSON record to scoped_memory.write("last_run", …)
  │        Record: { task, agent, timestamp, subtask_results }
  │
  └─ 8. Combine all outputs → SubTaskResult
```

---

## Events published

| Variant | When |
|---------|------|
| `AgentEvent::TaskAssigned` | At the start of every run |
| `AgentEvent::ModelRequest` | Before each worker's model call |
| `AgentEvent::WorkerResult` | Worker completes successfully |
| `AgentEvent::WorkerBlocked` | Worker fails / reports a blocker |
| `AgentEvent::TaskFailed` | Unrecoverable error (e.g. planning fails) |
| `AgentEvent::TaskCompleted` | All workers done, result assembled |

These events are consumed by `crates/web/src/metrics.rs` (dashboard counters)
and `crates/tui/` (operator terminal view).

---

## Memory integration

After each run, the sub-orchestrator writes a synthesis record to its scoped
memory namespace under the key `last_run`:

```json
{
  "task": "Add dark-mode toggle …",
  "agent": "feature-dev",
  "timestamp": "2026-05-20T14:03:00Z",
  "subtask_results": [
    { "worker": "frontend", "subtask": "…", "output": "…", "blocked": false },
    { "worker": "backend",  "subtask": "…", "output": "…", "blocked": false }
  ]
}
```

Read it back:

```bash
agent007 memory read feature-dev last_run
```

Or via MCP:

```json
{ "tool": "agent007_memory_read", "arguments": { "scope": "feature-dev", "key": "last_run" } }
```

---

## Configuration tips

### Limiting recursion depth

`SubOrchestrator::new` accepts `depth` (current) and `max_depth`.
The CLI and MCP handler hard-code `depth=0, max_depth=3`.
Reduce `max_depth` if you want shallower delegation trees.

### Model selection

Add `model = "claude-opus-4"` (or any router alias) to the agent TOML to
pin a particular model for that agent's planning and worker calls.

### Zone enforcement

Agents honour the same zone rules as the main `run` command.
Declare `[zones]` in the agent TOML to restrict which paths workers
may touch. The orchestrator checks zones before dispatching.

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| `Agent 'x' not found` | No TOML file for that name | Run `agent007 agent list` |
| `allowed_workers is empty` | Sub-orchestrator TOML missing `allowed_workers` | Edit the TOML |
| Workers produce generic output | Personas for `allowed_workers` names don't exist | `dispatch_parallel` falls back to an empty system prompt when no persona matches; add persona files for each worker name, or use built-in names (`coder`, `reviewer`, `planner`) |
| `MaxDepthExceeded` | Agent called itself recursively | Reduce max_depth or check TOML |
