# Configuration

## config.toml

Main config at `~/.agent007/config.toml`. Created by `agent007 init`.

Provider onboarding direction:

- **Current:** direct config/env setup is supported and remains valid
- **Planned:** dashboard-first provider onboarding/validation will manage the same runtime configuration model instead of replacing it
- **Always supported:** headless/manual setups via `config.toml` and environment variables

```toml
[core]
max_agents = 8              # Maximum concurrent agents
task_queue_capacity = 256   # Task queue depth

[models]
default = "claude"          # Default provider: "claude", "codex", "ollama"

[models.claude]
default_model = "claude-sonnet-4-6"

[models.codex]
default_model = "gpt-5.3-codex"

[models.ollama]
base_url = "http://localhost:11434"
default_model = "llama3"

# Route different task categories to specific models
[models.routing]
code_completion = "codex"      # Code autocomplete tasks
reasoning       = "claude"     # Complex reasoning tasks
fast_local      = "ollama"     # Quick tasks using local model
sensitive       = "ollama"     # Tasks that shouldn't leave your machine
default         = "claude"     # Fallback
```

All fields are optional — agent007 uses sensible defaults if omitted.

## Provider setup modes

Today, real provider-backed standalone runtime availability is determined from:

1. `ANTHROPIC_API_KEY`
2. `OPENAI_API_KEY`
3. reachable `[models.ollama]` config

If none are available, agent007 remains usable in **hosted-MCP** mode, where the connected host/editor LLM executes reasoning and tool orchestration through MCP. For tests and demos, `AGENT007_DRY_RUN=1` can also enable standalone execution with the mock provider; it is not a real model provider setup.

Planned UX direction:

- the **web dashboard** becomes the primary setup and validation surface for providers
- manual config/env setup remains compatible
- future OpenAI-compatible endpoint setup should also be manageable from dashboard

---

## hooks.toml

Hook config at `~/.agent007/hooks/hooks.toml` (global) or `.agent007/hooks/hooks.toml` (project-local). Project-local takes priority.

Created by `agent007 init` with empty values.

```toml
# Fire before any agent run begins
pre_agent_run = ""

# Fire after any agent run completes
post_agent_run = ""

# Fire before a tool call
pre_tool_call = ""

# Fire after a tool call
post_tool_call = ""

# Fire when a memory key is written
on_memory_write = ""

# Fire when a skill is executed
on_skill_execute = ""

# Fire after a task is marked complete (via agent007_record_tokens)
post_task_complete = ""
```

### Environment variables available to hook commands

| Variable | Set by event |
|----------|-------------|
| `HOOK_SKILL` | `on_skill_execute` |
| `HOOK_ARGS` | `on_skill_execute` |
| `HOOK_KEY` | `on_memory_write` |
| `HOOK_SCOPE` | `on_memory_write` |

### Examples

```toml
# Desktop notification when a task finishes
post_task_complete = "osascript -e 'display notification \"Task complete\" with title \"agent007\"'"

# Log all skill runs
on_skill_execute = "echo \"$(date): $HOOK_SKILL $HOOK_ARGS\" >> ~/agent007-skills.log"

# Sync memory to a backup location
on_memory_write = "rsync -q ~/.agent007/memory/ ~/Backup/agent007-memory/"
```

---

## zones.toml

Optional access control for file paths. Prevents agent tools from reading or writing outside allowed zones.

```toml
[[zones]]
path = "/etc"
allow_read = false
allow_write = false
allow_execute = false

[[zones]]
path = "~/.ssh"
allow_read = false
allow_write = false
allow_execute = false
```

Check a path: `agent007_zone_check path="/etc/passwd" operation="read"`

---

## Memory backends

By default, memory uses a flat file store (JSON files under `~/.agent007/memory/`).

LanceDB vector search is available when an embedding model is configured (configuration TBD — see `crates/memory/vectordb`).

---

## MCP downstream servers

To connect agent007 to other MCP servers (making their tools available via `agent007_mcp_tools_list` and `agent007_mcp_tool_call`):

```toml
[mcp.servers.my-server]
command = "/path/to/my-mcp-server"
args = ["--flag"]
```

---

## LSP configuration

LSP context injection can be configured in `config.toml`:

```toml
[lsp]
enabled = true
inject_for_categories = ["code_completion", "reasoning", "code", "dev", "frontend"]

[lsp.servers.rust_analyzer]
command = "rust-analyzer"
args = []
```

Behavior:
- `enabled=false` disables LSP context injection entirely.
- `inject_for_categories` controls which task and skill categories receive LSP context. Defaults cover routing categories plus built-in code, dev, and frontend skills.
- `servers` is a map keyed by server name.

You can also manage this from the dashboard:
- `GET /api/lsp/config`
- `POST /api/lsp/config`
- `DELETE /api/lsp/config`

When both global and project configs exist, project-level values override global defaults.

---

## ETR and tool-admission policy

For deterministic low-latency extraction/query helpers, use ETR built-ins via:
- `agent007_etr_list`
- `agent007_etr_call`

Policy for core vs optional/plugin tools:
- [docs/etr-tool-admission-policy.md](etr-tool-admission-policy.md)

## Repo intelligence index

agent007 writes repo-intelligence lookups to `.agent007/runtime/repo_index_v2.redb`. ETR and MCP graph queries prefer this bounded index for symbol lookup, callers/callees, usage graph, doc links, and prompt-context bundles. The older `repo_graph_v1.json` path is kept for compatibility with legacy APIs, but new code should query `RepoIndex` instead of loading the full JSON graph.

