# agent007

> Multi-agent AI orchestration — MCP server, CLI, skills, workflows, and memory. Works with Claude Code, Cursor, Codex, GitHub Copilot, and Zed.

## What it does

agent007 runs as an MCP server that gives your AI editor a broad orchestration tool layer:

- **Skills** — reusable prompt templates triggered by `/slash-commands`
- **Workflows** — multi-step pipelines (TDD, code review, feature delivery, SPARC)
- **Memory** — scoped key-value store + LanceDB vector search, persisted to `~/.agent007/`
- **Personas** — swap agent identities with different system prompts and preferred models
- **ModelRouter** — route tasks to the right model (code, reasoning, fast, sensitive)
- **Hooks** — auto-execute shell commands on lifecycle events
- **Learning** — passive feedback recording → future PromptOptimizer
- **Git agent** — AI-powered branch, commit, PR, and impact analysis
- **Web dashboard** — live run/task/memory inspector at `http://localhost:8007`, with standalone task execution when a local provider such as Ollama is configured
- **Operator TUI** — `agent007 tui` opens a terminal console for sessions, run detail, approvals, recent errors, and structured run messages
- **Dashboard-first provider UX (planned)** — provider health, setup validation, and onboarding will be centered in the web dashboard while preserving config/env-based setup for headless use
- **LSP context controls** — configure LSP servers + category injection from config and dashboard (`/api/lsp/config`)
- **ETR built-ins** — low-latency deterministic extraction/query/metrics tools to reduce shell+parsing overhead

---

## Install

Current release strategy: GitHub Releases plus a curl-based installer. Homebrew,
`apt`, and other package-manager distribution are intentionally deferred while
the project is still moving quickly. See [docs/release-strategy.md](docs/release-strategy.md).

Install the latest prebuilt release (Linux x86_64):

```bash
curl -fsSL https://raw.githubusercontent.com/danieldear/agent007/main/scripts/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/danieldear/agent007/main/scripts/install.sh | bash -s -- --version v0.1.0
```

Direct install alternatives:

```bash
cargo install --path crates/cli
```

`cargo install --git ...` and local cargo builds now trigger frontend asset build automatically for `agent007-web` when `static/dist` is missing or stale, and the compiled binary serves an embedded dashboard bundle so `agent007 serve` works after install.

macOS users can install from source:

```bash
cargo install --git https://github.com/danieldear/agent007.git agent007
```

Release automation currently publishes Linux artifacts only; macOS is source-install for now.

Or build from source:

```bash
cargo build --release
# binary at target/release/agent007
```

---

## Quick start

```bash
# 1. Initialize — creates ~/.agent007/, writes config, registers MCP with your editors
agent007 init

# 2. Start the MCP server (editors connect automatically via their MCP config)
agent007 serve

# 3. Or run a one-shot task from the terminal
agent007 run "Refactor the auth module to use JWT"
```

---

## Known issues

- Workflow approval ownership and dashboard resume behavior:
  externally initiated runs are read-only in the dashboard, while dashboard-owned
  standalone runs can still be resumed there. See [docs/known-issues.md](docs/known-issues.md)

---

## CLI reference

### `agent007 init`

Sets up `~/.agent007/` directory structure, writes `config.toml` and `hooks.toml`, and registers the MCP server with your IDE(s).

```
agent007 init [OPTIONS]

Options:
  --force      Re-run even if already initialized (overwrites existing config)
  --global     Register globally in ~/.claude/ instead of project .claude/
  --claude     Set up Claude Code integration only
  --cursor     Set up Cursor integration only
  --codex      Set up Codex integration only
  --copilot    Set up GitHub Copilot (VS Code) integration only
  --zed        Set up Zed integration only
  --no-ide     Skip all IDE integration — only create .agent007/ dirs
```

By default (no flags), all supported IDEs are configured.

### `agent007 serve`

Starts the MCP server on stdio (for IDE connection) and the web dashboard. When
standalone execution is available, dashboard task runs persist the full model
output in the selected run detail view.

```
agent007 serve [OPTIONS]

Options:
  --port <PORT>     Web dashboard port [default: 8007]
  --no-dashboard    Disable web dashboard (MCP-only mode)
```

### `agent007 run <TASK>`

Run a task through the full agent stack (ModelRouter → skill dispatch → memory RAG → response).

```bash
agent007 run "Write unit tests for src/auth.rs"
```

### `agent007 skill`

Manage skills — reusable prompt templates stored as `.md` files with YAML frontmatter.

```
agent007 skill list                              List all available skills
agent007 skill run <trigger> [args]             Execute a skill by trigger
agent007 skill install github:<owner>/<repo>/<path>   Install from GitHub
agent007 skill install https://example.com/skill.md   Install from URL
agent007 skill create                            Interactive skill creator
```

### `agent007 workflow`

Run multi-step agent pipelines.

```
agent007 workflow list                           List available workflows
agent007 workflow run <name> <task>             Run workflow synchronously
agent007 workflow start <name> <task>           Start hosted workflow session
agent007 workflow next <session>                Fetch next steps to execute
agent007 workflow submit <session> <step> <output>  Submit a step's output
agent007 workflow status <session>              Check session state
agent007 workflow approve <session>             Approve a pending gate
agent007 workflow resume <session>              Resume a paused workflow
```

### `agent007 agent`

Manage and run custom multi-agent definitions stored as TOML files in `~/.agent007/agents/`.

```
agent007 agent list                            List all registered agents
agent007 agent inspect <name>                  Show agent definition details
agent007 agent run <name> "<task>"             Run an agent on a task
agent007 agent create <name> [--type <type>]   Generate a new agent TOML stub
                             [--namespace <ns>]
```

See [docs/multi-agent.md](docs/multi-agent.md) for the full execution model,
TOML schema, and event reference.

### `agent007 persona`

```
agent007 persona list          List all personas
agent007 persona show <name>   Show persona details + system prompt
agent007 persona switch <name> Activate a persona
```

### `agent007 git`

AI-powered git operations.

```
agent007 git branch <description>    Create a branch from natural language
agent007 git commit                  Generate a commit message from diff
agent007 git pr                      Draft a PR description
agent007 git impact                  Analyze risk of current diff
```

### `agent007 checkpoint`

Stash-based checkpoints for safe experimentation.

```
agent007 checkpoint save <name>      Save current state
agent007 checkpoint list             List saved checkpoints
agent007 checkpoint restore <name>   Restore a checkpoint
```

### Other commands

```
agent007 simulate <template>   Run simulation templates
agent007 test                  Run the AI testing pipeline
agent007 audit                 Run a security/quality audit
agent007 replay <run-id>       Replay a past run
```

---

## MCP tools

These are available to your AI editor once `agent007 serve` is running.

### Core
| Tool | Description |
|------|-------------|
| `agent007_dispatch` | Dispatch a command-style request to run/skill/workflow tools (`$agent007 ...`) |
| `agent007_run` | Run a task through the full agent stack |
| `agent007_task_submit` | Submit a task to the orchestrator queue |
| `agent007_help` | Show available tools and capabilities |
| `agent007_health` | Health check — memory dir, skills count, personas |
| `agent007_config_show` | Show current config.toml as TOML |

### Skills
| Tool | Description |
|------|-------------|
| `agent007_skill_list` | List all skills with trigger, description, version |
| `agent007_skill_run` | Execute a skill by trigger with args |
| `agent007_skill_create` | Create a new skill file |
| `agent007_skill_wizard` | Interactive skill creation wizard |
| `agent007_skill_write_tests` | Generate tests for a skill |
| `agent007_skill_review_pr` | Run PR review skill |

### Workflows
| Tool | Description |
|------|-------------|
| `agent007_workflow_list` | List workflow YAML files |
| `agent007_workflow_run` | Run a workflow synchronously |
| `agent007_workflow_start` | Start a hosted workflow session |
| `agent007_workflow_next` | Fetch next ready steps |
| `agent007_workflow_submit_step` | Submit a step output |
| `agent007_workflow_status` | Inspect workflow session state |
| `agent007_workflow_approve` | Record approval decision |
| `agent007_workflow_resume` | Resume from a prior session |
| `agent007_workflow_plan` | Get execution plan without running |
| `agent007_workflow_create` | Save a new workflow YAML |
| `agent007_workflow_tdd` | Shortcut: start TDD workflow |
| `agent007_workflow_code_review` | Shortcut: start code-review workflow |

### Memory
| Tool | Description |
|------|-------------|
| `agent007_memory_read` | Read a value by scope + key |
| `agent007_memory_write` | Write a key/value to a scope |
| `agent007_memory_list` | List all keys in a scope |
| `agent007_context_compile` | Compile a task-scoped context bundle (repo brain + memory + files) |
| `agent007_repo_brain_refresh` | Refresh the persistent repo summary |

### Agents & Personas
| Tool | Description |
|------|-------------|
| `agent007_agent_run` | Run a named custom agent on a task (parallel worker dispatch, replan, memory synthesis) |
| `agent007_agent_create` | Create or browse agent archetypes |
| `agent007_persona_list` | List available personas |
| `agent007_persona_show` | Show persona details |
| `agent007_persona_switch` | Activate a persona |

### Git
| Tool | Description |
|------|-------------|
| `agent007_git_status` | Run git status |
| `agent007_git_diff` | Run git diff (staged + unstaged) |
| `agent007_git_log` | Show last N commits |
| `agent007_git_commit` | Create a commit with a message |

### Observability
| Tool | Description |
|------|-------------|
| `agent007_run_history` | List recent recorded runs |
| `agent007_run_show` | Show run metadata + event trace |
| `agent007_record_tokens` | Record actual token usage for a run (also writes FeedbackEntry to learning store) |
| `agent007_compact_output` | Compact noisy command output into high-signal summary |
| `agent007_budget_estimate` | Estimate prompt budget pressure |

### MCP passthrough
| Tool | Description |
|------|-------------|
| `agent007_mcp_tools_list` | List tools from downstream MCP servers |
| `agent007_mcp_tool_call` | Call a downstream MCP tool |

### Zones & access control
| Tool | Description |
|------|-------------|
| `agent007_zone_check` | Check if a path operation is allowed by zone rules |

### ETR (Embedded Tool Runtime)
| Tool | Description |
|------|-------------|
| `agent007_etr_list` | List available ETR tools and schemas |
| `agent007_etr_call` | Call an ETR tool with structured JSON input |

Built-in ETR tools include:
- `etr.json_extract`, `etr.json_query`, `etr.json_query_v2`
- `etr.text_extract`, `etr.table_select`, `etr.table_stats`, `etr.group_count`
- `etr.time_window_filter`, `etr.join_on_key`, `etr.metrics_summary`, `etr.delta_compare`
- `etr.workflow_status_summary`, `etr.workflow_outputs_index`, `etr.workflow_step_health`
- `etr.artifact_read`, `etr.logs_slice`, `etr.logs_correlate`
- `etr.glob`, `etr.file_stat`, `etr.grep`, `etr.diff`, `etr.csv_slice`, `etr.math`
- `etr.semantic_search_local`, `etr.policy_check`

---

## Skills system

Skills are Markdown files with YAML frontmatter. They live in `~/.agent007/skills/` (global) or `.agent007/skills/` (project-local).

```markdown
---
name: My Skill
trigger: /my-skill
description: Does something useful
model: claude-sonnet-4-6
version: "1.0.0"
---
You are a helpful assistant. Given: {{args}}

Do the following...
```

### Built-in skills (core set + optional specializations)

| Trigger | Description |
|---------|-------------|
| `/code-document` | Generate API docs, architecture docs, inline documentation |
| `/code-optimize` | Profile analysis and performance optimization suggestions |
| `/code-refactor` | Identify code smells and propose targeted improvements |
| `/code-security-audit` | Security audit covering OWASP, dependencies, threat modeling |
| `/code-test-gen` | Generate comprehensive test suites with edge cases |
| `/dev-architect` | Design system architecture from requirements |
| `/dev-debug` | Systematic debugging with hypothesis-driven investigation |
| `/dev-pr-review` | Thorough pull request review with actionable feedback |
| `/dev-tdd` | Test-driven development cycle (red-green-refactor) |
| `/meta-analyze-codebase` | Analyze codebase for tech stack, patterns, and architecture |
| `/meta-create-agent` | Guided wizard to create a custom agent persona |
| `/project-changelog` | Generate changelogs grouped by type from git history |
| `/project-plan` | Break features into tasks with estimates and dependencies |
| `/project-prd` | Product requirements document with user stories and constraints |
| `/project-release` | Version strategy, release notes, and rollback planning |
| `/frontend-designer` | High-quality frontend UI/component design and implementation |
| `/brainstorm` | Free-form ideation before PRD/architecture workflows |

### Install a skill

```bash
# From GitHub
agent007 skill install github:owner/repo/path/to/skill.md

# From URL
agent007 skill install https://example.com/my-skill.md
```

---

## Workflows

Workflows are YAML pipelines in `~/.agent007/workflows/`. Steps without dependencies run in parallel; steps with `depends_on` run sequentially.

| Workflow | Steps | Use for |
|----------|-------|---------|
| `tdd` | 3 | Red → Green → Refactor cycle |
| `code-review` | 4 | Security + performance + style in parallel, synthesized |
| `sparc` | 5 | Spec → Pseudocode → Architecture → Refinement → Completion |
| `log-analysis` | 4 | Error analysis + pattern detection + security check in parallel |
| `feature` | 17 | Full delivery: spec → arch → implement → review → test → release |
| `ideation` | 7 | Research → PRD → architecture → project plan |
| `security-audit` | 5 | OWASP + secrets + threat model + dependencies in parallel |
| `brainstorm` | lightweight | Ideation → approval → PRD/doc output |

---

## Hooks

Auto-execute shell commands at lifecycle events. Configure in `~/.agent007/hooks/hooks.toml` (project `.agent007/hooks/hooks.toml` takes priority):

```toml
pre_agent_run     = ""
post_agent_run    = ""
pre_tool_call     = ""
post_tool_call    = ""
on_memory_write   = "echo 'Memory written: $HOOK_KEY'"
on_skill_execute  = "echo 'Skill: $HOOK_SKILL'"
post_task_complete = "notify-send 'agent007' 'Task complete'"
```

---

## Memory

Memory is persisted to `~/.agent007/memory/`. Scopes:

- `global` — cross-session facts
- `user` — user preferences
- `project` — project-specific notes
- `learning` — passive FeedbackEntry records (written by `agent007_record_tokens`)
- Any custom namespace

Vector search via LanceDB is available when configured.

---

## IDE setup

| IDE | Config location | How to register |
|-----|-----------------|-----------------|
| Claude Code | `.claude/mcp.json` | `agent007 init --claude` |
| Cursor | `.cursor/mcp.json` | `agent007 init --cursor` |
| Codex | `.codex/config.toml` | `agent007 init --codex` |
| GitHub Copilot (VS Code) | `.vscode/mcp.json` | `agent007 init --copilot` |
| Zed | `~/.config/zed/settings.json` | `agent007 init --zed` |

`agent007 init` (no flags) registers all of the above at once.

### Codex command-style dispatch

From Codex, you can use one MCP tool for slash-like ergonomics:

```
agent007_dispatch command="$agent007 wf tdd add login rate limiting"
agent007_dispatch command="$agent007 skill /brainstorm onboarding ideas"
agent007_dispatch command="$agent007 run refactor auth module"
```

This is additive convenience; direct tools (`agent007_workflow_run`, `agent007_skill_run`, `agent007_run`) still work.

---

## Configuration

Full config lives at `~/.agent007/config.toml`. See [docs/configuration.md](docs/configuration.md) for the complete schema.

LSP can be configured via:
- file config: `[lsp]` block in `config.toml`
- dashboard API: `GET/POST/DELETE /api/lsp/config`

---

## Architecture

See [docs/architecture.md](docs/architecture.md) for the crate map and data flow.

---

## Docs

- [docs/architecture.md](docs/architecture.md) — crate map, data flow, key interfaces
- [docs/skills.md](docs/skills.md) — skills system deep-dive
- [docs/workflows.md](docs/workflows.md) — workflow reference
- [docs/configuration.md](docs/configuration.md) — config.toml + hooks.toml schema
- [docs/release-strategy.md](docs/release-strategy.md) — release policy and channels
- [docs/known-issues.md](docs/known-issues.md) — tracked product gaps

---

## Governance

- [LICENSE](LICENSE)
- [SECURITY.md](SECURITY.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- [SUPPORT.md](SUPPORT.md)
- [CHANGELOG.md](CHANGELOG.md)
