# Multi-Agent Execution

agent007 ships a full multi-agent orchestration layer.  
A single *sub-orchestrator* agent decomposes a high-level task into sub-tasks,
dispatches them to worker personas **in parallel**, handles blockers with a
second replan round, and synthesises a combined result — all while publishing
structured events to the dispatcher so the dashboard and TUI stay live.

---

## Concepts

| Term | Description |
|------|-------------|
| **AgentDef** | TOML file in `~/.agent007/agents/` that describes an agent (type, system prompt, allowed workers, zones, memory namespace, …) |
| **AgentType** | `worker` or `sub-orchestrator` |
| **SubOrchestrator** | Rust struct that owns the decompose → dispatch → replan → synthesise loop |
| **WorkerOutput** | Per-subtask result: output string + optional blocker reason |
| **SubTaskResult** | Final result returned to the caller: combined `output`, `blockers`, `files_changed`, `tests_passed` |
| **Dispatcher** | Broadcast channel; publishes `AgentEvent` variants consumed by the dashboard metrics engine and TUI |

---

## Agent TOML schema

```toml
# ~/.agent007/agents/my-agent.toml
name             = "my-agent"
type             = "sub-orchestrator"     # or "worker"
description      = "Does X, Y, Z"
system_prompt    = "You are …"
memory_namespace = "my-agent"            # optional, defaults to name

# Sub-orchestrators only
allowed_workers  = ["frontend", "backend", "qa"]

# Optional zone rules
[zones]
readonly  = ["docs/"]
forbidden = [".env", "secrets/"]
```

Generate a stub with the CLI:

```bash
agent007 agent create my-agent --type sub-orchestrator --namespace my-ns
# writes ~/.agent007/agents/my-agent.toml
```

---

## CLI usage

```
agent007 agent list                     List all registered agents
agent007 agent inspect <name>           Show agent definition details
agent007 agent run <name> "<task>"      Run an agent on a task
agent007 agent create <name> [OPTIONS]  Generate a new agent TOML stub
```

### Example

```bash
# Create a sub-orchestrator that manages frontend and backend workers
agent007 agent create feature-dev --type sub-orchestrator --namespace feature-dev

# Edit ~/.agent007/agents/feature-dev.toml to fill in system_prompt + allowed_workers

# Run it
agent007 agent run feature-dev "Add dark-mode toggle to the settings page"
```

Output:

```
🤖 Running agent 'feature-dev' …
   Task: Add dark-mode toggle to the settings page

[combined output from all workers]

📂 Files changed:
  • src/settings/ToggleDarkMode.tsx
  • src/styles/tokens.css
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
