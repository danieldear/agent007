# Product Requirements Document: agent007

**Version:** 1.1  
**Status:** Current State (as-built)  
**Last Updated:** 2026-05-03  
**Owner:** agent007 project maintainers

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Problem Statement](#2-problem-statement)
3. [Target Users & Personas](#3-target-users--personas)
4. [Goals & Success Metrics](#4-goals--success-metrics)
5. [Non-Goals](#5-non-goals)
6. [Feature Requirements](#6-feature-requirements)
   - 6.1 [Skills System](#61-skills-system)
   - 6.2 [Workflow Engine](#62-workflow-engine)
   - 6.3 [Memory & Context](#63-memory--context)
   - 6.4 [Hooks & Events](#64-hooks--events)
   - 6.5 [Learning & Optimization](#65-learning--optimization)
   - 6.6 [IDE Integration](#66-ide-integration)
   - 6.7 [CLI](#67-cli)
   - 6.8 [Model Routing](#68-model-routing)
   - 6.9 [Personas](#69-personas)
   - 6.10 [Git Agent](#610-git-agent)
   - 6.11 [Zones (Access Control)](#611-zones-access-control)
   - 6.12 [Web Dashboard & TUI](#612-web-dashboard--tui)
   - 6.13 [MCP Server](#613-mcp-server)
7. [Technical Requirements](#7-technical-requirements)
8. [Known Limitations & Future Work](#8-known-limitations--future-work)
9. [Glossary](#9-glossary)

---

## 1. Executive Summary

**agent007** is an AI orchestration platform for software engineers who work inside AI-assisted editors (Claude Code, Cursor, Codex, GitHub Copilot). It runs as a local daemon that exposes 44 tools to any editor supporting the Model Context Protocol (MCP), enabling reusable prompt skill libraries, multi-step agent workflows, persistent cross-session memory, and passive learning from usage patterns—all without leaving the editor.

agent007 ships as a single Rust binary that acts simultaneously as:
- An **MCP server** (stdio transport) consumed by AI editors
- A **CLI tool** for direct invocation of all capabilities
- A **web dashboard** (port 8007) for real-time monitoring and management
- An **integration bridge** that auto-configures Claude Code, Cursor, Codex, GitHub Copilot, and Zed

The system is composed of 22 focused crates, including newer extension and collaboration/runtime surfaces (`extensions`, `sharing`, `p2p`, and others in the workspace).

### 2026-05-03 Addendum

Recent baseline additions now included in the product surface:

1. Extension platform (`preview/install/list`) with adapters for local, GitHub, npm MCP, OpenAPI, and marketplace-style sources.
2. MCP server registry and RAG source management APIs exposed in the dashboard.
3. Tool registry search/import/test/approve lifecycle.
4. Memory observability stats endpoint and dashboard visibility.
5. Runtime learning workers active in both CLI `run` and `serve` flows.

---

## 2. Problem Statement

AI editors are powerful but stateless and generic. Engineers who use them daily face three compounding problems:

**1. Prompts are ephemeral and not reusable.**  
Every new chat session requires re-explaining context, coding standards, or preferred approaches. There is no mechanism to save, version, share, or invoke a prompt as a first-class artifact. Teams rediscover and rewrite the same prompts repeatedly.

**2. Multi-step agentic tasks have no structure.**  
Complex tasks—security audits, feature delivery, TDD cycles—require many sequential or parallel agent invocations with conditional logic, approval gates, and result aggregation. AI editors provide no native DAG execution or workflow coordination layer. Engineers either run steps manually or write throwaway scripts.

**3. Nothing is remembered across sessions.**  
Project-specific conventions, prior decisions, architectural constraints, and learned preferences disappear between sessions. Each conversation starts from zero. As a result, AI output quality degrades over time relative to a growing codebase, rather than improving.

agent007 solves all three problems with a unified, local-first orchestration layer that integrates directly into the tools engineers already use.

---

## 3. Target Users & Personas

### Primary Users

**The AI-Native Engineer**  
- Uses Claude Code, Cursor, Codex, or GitHub Copilot as their primary development interface
- Runs 10–50 AI interactions per day
- Wants consistent, high-quality AI assistance without constant re-prompting
- Values automation but needs to remain in control of critical decisions

**The Team Tech Lead**  
- Responsible for maintaining engineering standards across a team
- Wants to codify and share prompt libraries (skills) and review workflows
- Needs visibility into how AI tools are being used and what they produce

### Secondary Users

**The Workflow Automation Engineer**  
- Builds and maintains CI/CD-adjacent automation
- Uses agent007's workflow YAML engine to orchestrate multi-agent pipelines
- Integrates agent007 with existing toolchains via hooks and the CLI

**The AI Researcher / Prompt Engineer**  
- Evaluates and iterates on prompt quality systematically
- Uses the learning system and simulation/replay capabilities to measure improvement

---

## 4. Goals & Success Metrics

### Goals

| # | Goal |
|---|------|
| G1 | Enable engineers to define, share, and invoke reusable prompt skills from within their AI editor in under 5 seconds |
| G2 | Allow complex multi-step agent tasks (up to N parallel steps) to run as structured workflows with dependency ordering and approval gates |
| G3 | Persist project and user context across all sessions so AI quality improves over time rather than resetting |
| G4 | Passively learn from usage patterns and automatically improve prompt quality without manual intervention |
| G5 | Integrate with all major AI editors via zero-friction auto-configuration |
| G6 | Operate entirely locally with no required cloud dependency for core features |

### Success Metrics

| Metric | Target |
|--------|--------|
| Skill invocation latency (p95) | < 2 seconds from trigger to first token |
| Workflow step scheduling overhead | < 100 ms per step |
| Memory read latency (scoped key-value) | < 50 ms |
| IDE auto-configuration success rate | 100% for supported editors on first `agent007 init` |
| MCP tool surface | ≥ 44 tools exposed and discoverable |
| Skill install time (from GitHub or HTTPS) | < 10 seconds |

---

## 5. Non-Goals

The following are explicitly out of scope for the current production system:

- **Cloud sync or multi-device memory:** Memory is local-only; no cloud backend is provided or planned in the current version
- **Native GUI application:** The web dashboard is a monitoring and management interface, not a primary interaction surface
- **LLM hosting:** agent007 routes to external model providers (Anthropic, OpenAI, Ollama); it does not host or fine-tune models
- **Full IDE plugin/extension:** Integration is via MCP configuration files and the MCP protocol; no editor-native plugin code is maintained
- **General-purpose task runner:** While workflows are powerful, agent007 is not a replacement for make, just, or CI systems
- **Multi-user / team server mode:** The current system is single-user, local-first; team sharing is achieved by committing skill/workflow YAML files to source control

---

## 6. Feature Requirements

### 6.1 Skills System

Skills are the fundamental unit of reusable AI capability in agent007. A skill is a Markdown file with YAML frontmatter defining its metadata and a Tera-templated prompt body.

#### Skill Authoring

- Each skill file must declare: `trigger` (e.g., `/code-document`), `name`, `description`, `model` (optional), and `version`
- Skill bodies are Tera templates; the special variable `{{args}}` carries user-provided arguments at invocation time
- Skills may reference memory, prior run context, or project-specific variables via template interpolation

#### Skill Discovery & Resolution

- **Built-in skills (core set + optional specializations):** Shipped with the agent007 binary; always available with no configuration:

  | Trigger | Purpose |
  |---------|---------|
  | `/code-document` | Generate API docs, architecture docs, and inline documentation |
  | `/code-optimize` | Profile analysis and performance optimization suggestions |
  | `/code-refactor` | Identify code smells and propose targeted improvements |
  | `/code-security-audit` | Security audit covering OWASP, dependencies, and threat modeling |
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

- **Project-local skills:** Stored in `.agent007/skills/` at the repository root; override built-ins with the same trigger
- **User-global skills:** Stored in `~/.agent007/skills/`; available across all projects
- **Resolution order:** Project-local → User-global → Built-in (first match wins)

#### Skill Installation

- Install from GitHub: `agent007 skill install github:<owner>/<repo>` fetches skills from a remote repository
- Install from URL: `agent007 skill install https://<url>` fetches a single skill file
- Installed skills land in `~/.agent007/skills/` by default; `--project` flag targets `.agent007/skills/`

#### Skill Invocation

- Via MCP tool: `agent007_skill_run` with `trigger` and optional `args` parameters
- Via CLI: `agent007 skill run <trigger> [args]`
- Skills are invocable from within any AI editor that has the agent007 MCP server configured

#### Skill Management CLI

- `agent007 skill list` — enumerate all available skills with source (built-in / project / global)
- `agent007 skill install <source>` — install a skill from GitHub or HTTPS
- `agent007 skill create` — interactive wizard to scaffold a new skill file

---

### 6.2 Workflow Engine

Workflows are YAML-defined directed acyclic graphs (DAGs) of agent steps. They enable complex, multi-step agentic tasks to run with dependency ordering, parallelism, and human approval gates.

#### Workflow Definition

A workflow YAML file declares:
- `name` and `description` at the top level
- A `steps` array where each step specifies: `id`, `agent` (persona name), `prompt` (Tera template with `{{task}}`), `output` (variable name), and optional `depends_on` (list of step IDs)
- Steps without `depends_on` are eligible to run in parallel immediately
- Steps with `depends_on` are scheduled after all listed predecessors complete

#### Built-in Workflows (7)

| Workflow | Description |
|----------|-------------|
| `tdd` | Red → Green → Refactor TDD pipeline |
| `sparc` | Spec → Pseudocode → Architecture → Refinement → Completion |
| `code-review` | Parallel security, performance, and style review with synthesized findings |
| `log-analysis` | Parallel log analysis team with aggregated final report |
| `feature` | Full-cycle feature delivery: ideation → spec → architecture → implementation → review → tests → docs → release |
| `ideation` | Research → PRD → architecture → project planning pipeline |
| `security-audit` | Parallel OWASP, secrets, threat model, and dependency scanning with severity-ranked report |

#### Workflow Execution Modes

- **Direct execution:** `agent007_workflow_run` submits a workflow and returns a JSON object with each step's output on completion
- **Hosted-MCP execution:** `agent007_workflow_start` / `agent007_workflow_next` / `agent007_workflow_submit_step` enable a conversational step-by-step mode where the host LLM drives execution and submits outputs back through the MCP server; agent007 manages state, approvals, retries, and routing between steps
- **Plan-only mode:** `agent007_workflow_plan` returns the full execution plan (step prompts, personas, dependency graph) without running anything; the host LLM can execute steps itself

#### Approval Gates

- Any step can be configured as an approval gate; execution pauses and the user receives a prompt to `approve`, `deny`, or `edit` the step's output before the workflow continues
- Approval decisions are recorded via `agent007_workflow_approve`
- Denied steps halt the workflow; edited steps use the user-provided content as the step output

#### Workflow Persistence & Resume

- Workflow state is persisted to `~/.agent007/sessions/` as JSON artifacts (`workflow-request.json`, `workflow-state.json`)
- A paused or interrupted workflow can be resumed from a prior session via `agent007_workflow_resume <session-id>`

#### Workflow Management CLI

- `agent007 workflow list` — enumerate available workflows
- `agent007 workflow run <name> <task>` — execute a workflow
- `agent007 workflow resume <session>` — resume a paused workflow

---

### 6.3 Memory & Context

agent007 provides a persistent, scoped key-value store and a vector search layer so context accumulates across sessions.

#### Scoped Key-Value Store

- **Scopes:** `global`, `user`, `project`, and any arbitrary custom namespace
- Storage: flat files at `~/.agent007/memory/<scope>/`
- Operations exposed as MCP tools: `agent007_memory_read`, `agent007_memory_write`, `agent007_memory_list`
- Typical uses: storing architectural decisions, project conventions, preferred patterns, prior run summaries, persona preferences

#### Vector Search (LanceDB)

- A LanceDB-backed semantic search index is maintained alongside the key-value store
- Enables fuzzy/semantic retrieval of prior context ("what did we decide about auth?")
- Requires an embedding model to be configured; functions as a fallback to exact key-value lookup when not configured

#### Context Compilation

- `agent007_context_compile` assembles a task-scoped context bundle: repo brain summary, relevant files, memory notes, and recent run history into a single rendered context block for consumption by a skill or workflow
- `agent007_repo_brain_refresh` distills the current repository into a persistent summary saved to project memory

#### Budget Management

- `agent007_budget_estimate` estimates prompt budget pressure for a task or text block and recommends whether to use full, compact, or aggressive context
- `agent007_compact_output` compacts noisy command output (e.g., `cargo test`, `git diff`) into a high-signal summary, recording both raw and compact artifacts

#### Session History

- All run history is stored in `~/.agent007/sessions/` and queryable via `agent007_run_history` and `agent007_run_show`
- Sessions track: workflow state, skill invocations, token usage, model, and output artifacts

---

### 6.4 Hooks & Events

Hooks allow external shell commands or scripts to be triggered automatically at key points in the agent007 lifecycle.

#### Hook Configuration

- Hooks are declared in `hooks.toml` at the project root or `~/.agent007/hooks.toml` for global hooks
- Each hook entry specifies: the event name, the shell command to execute, and optional conditions

#### Supported Events (7)

| Event | When it fires |
|-------|--------------|
| `PreAgentRun` | Before any agent task begins execution |
| `PostAgentRun` | After an agent task completes |
| `PreToolCall` | Before any MCP tool is invoked |
| `PostToolCall` | After any MCP tool returns |
| `OnMemoryWrite` | When a value is written to the memory store |
| `OnSkillExecute` | When a skill is invoked |
| `PostTaskComplete` | When a full task (skill or workflow) finishes |

#### Hook Use Cases

- Logging all tool calls to a file for audit
- Sending a notification (Slack, macOS notification) when a long workflow completes
- Running `git add -p` after a `PostAgentRun` event
- Triggering lint or test runs after code-generation tasks

---

### 6.5 Learning & Optimization

agent007 passively records feedback from every completed run and uses it to drive prompt improvement over time.

#### Feedback Recording

- `agent007_record_tokens` is called after any skill or workflow step completes in hosted-MCP mode; it records: `run_id`, actual `tokens` used, `model` name, and optionally the `output` text
- When `output` is provided, it is saved to project memory for future context reuse, reducing repeated analysis across sessions
- Recorded data forms `FeedbackEntry` objects stored in the `LearningStore`

#### Prompt Optimization

- A `PromptOptimizer` component analyzes accumulated `FeedbackEntry` data to identify patterns in high- and low-quality outputs
- The optimizer generates suggested prompt improvements for skills that appear frequently in feedback data
- **Current state:** The optimizer scaffold exists; automatic triggering (e.g., at every N=20 entries) is not yet implemented — optimization currently requires manual invocation

---

### 6.6 IDE Integration

agent007 auto-configures itself as an MCP server in all supported AI editors when `agent007 init` is run in a project or globally.

#### Supported Editors

| Editor | Config location | Integration mechanism |
|--------|-----------------|-----------------------|
| Claude Code | `.claude/` | MCP server config injected into Claude's config directory |
| Cursor | `.cursor/` | MCP server config injected into Cursor's config directory |
| Codex | `.codex/` | MCP server config injected into Codex's config directory |
| GitHub Copilot (VS Code) | `.vscode/mcp.json` | MCP server entry written to workspace MCP config |
| Zed | `~/.config/zed/` | MCP server config written to Zed's global settings |

#### Integration Behavior

- `agent007 init` detects which editors are installed/configured and writes the appropriate MCP server config entry for each
- The agent007 binary is registered as an MCP server running in stdio mode
- All 44 MCP tools become immediately available in the editor's AI assistant after config is written and the editor reloads
- Project-local init (`.agent007/` directory) coexists with user-global config; project settings take precedence

#### MCP Tool Surface

The 44 tools exposed to editors are organized into functional groups:

- **Core orchestration:** `agent007_run`, `agent007_task_submit`
- **Skills:** `agent007_skill_list`, `agent007_skill_run`, plus one dedicated tool per built-in skill (e.g., `agent007_skill_code_document`, `agent007_skill_dev_tdd`)
- **Workflows:** `agent007_workflow_list`, `agent007_workflow_run`, `agent007_workflow_start`, `agent007_workflow_next`, `agent007_workflow_submit_step`, `agent007_workflow_status`, `agent007_workflow_approve`, `agent007_workflow_resume`, `agent007_workflow_plan`, `agent007_workflow_create`, plus one dedicated tool per built-in workflow
- **Memory:** `agent007_memory_read`, `agent007_memory_write`, `agent007_memory_list`
- **Context:** `agent007_context_compile`, `agent007_repo_brain_refresh`, `agent007_budget_estimate`, `agent007_compact_output`
- **Personas:** `agent007_persona_list`, `agent007_persona_show`, `agent007_persona_switch`, `agent007_agent_create`
- **Git:** `agent007_git_status`, `agent007_git_diff`, `agent007_git_log`, `agent007_git_commit`
- **System:** `agent007_health`, `agent007_config_show`, `agent007_zone_check`, `agent007_run_history`, `agent007_run_show`, `agent007_record_tokens`

---

### 6.7 CLI

The agent007 CLI is the primary management interface for direct, non-editor usage.

#### Top-Level Subcommands

| Subcommand | Purpose |
|------------|---------|
| `run` | Execute a task through the full agent stack (ModelRouter, memory, skills, hooks) |
| `init` | Initialize agent007 in the current project and configure IDE integrations |
| `serve` | Start the MCP server (stdio) and optional web dashboard |
| `skill` | Manage skills: list, install, create, run |
| `workflow` | Manage and execute workflows |
| `persona` | List, show, and switch active personas |
| `git` | Git agent commands: branch, commit, PR creation, impact analysis |
| `checkpoint` | Save and restore session checkpoints |
| `simulate` | Run simulated agent tasks for testing without making real model calls |
| `test` | Run the agent007 testing framework against a skill or workflow |
| `audit` | Security and usage audit reporting |
| `replay` | Replay a prior session or run for debugging or demonstration |

---

### 6.8 Model Routing

agent007 routes model requests to the appropriate provider and model based on task characteristics, avoiding the need for users to manually specify a model for every call.

#### Routing Categories

| Category | Description | Example providers |
|----------|-------------|-------------------|
| `code_completion` | Short-context, latency-sensitive code completions | Codex, fast local |
| `reasoning` | Complex multi-step reasoning, planning | Claude (Sonnet/Opus) |
| `fast_local` | Low-latency, privacy-sensitive, offline-capable tasks | Ollama |
| `sensitive` | Tasks involving secrets, credentials, or private data | Local-only models |
| `default` | General-purpose fallback | Configured default provider |

#### Routing Configuration

- Routing is defined in `RoutingConfig` within agent007's configuration file
- Each category maps to a provider name, model name, and optional fallback chain
- Skills and workflows can declare a preferred model category in their frontmatter; the router fulfills the request using the mapped provider

#### Token Tracking

- Every model response carries `input_tokens` and `output_tokens` counts
- Token usage is accumulated per session and per run for budget reporting and learning feedback

---

### 6.9 Personas

Personas give agent steps a defined identity, system prompt, and tool set, enabling consistent specialist behavior across workflows.

#### Persona Definition

- Each persona is a Markdown file with YAML frontmatter: `name`, `description`, `preferred_model`, and `allowed_tools`
- The body of the file is the persona's system prompt
- Built-in personas (10+) cover common specialist roles: Researcher, Architect, Planner, Reviewer, Security Auditor, Performance Optimizer, Debugger, TDD Coach, and others

#### Persona Resolution

- **Project-local personas:** `.agent007/personas/`
- **User-global personas:** `~/.agent007/personas/`
- **Built-in personas:** Shipped with the binary
- Resolution order mirrors the skills system: project-local → user-global → built-in

#### Persona Operations

- `agent007_persona_list` — enumerate all available personas
- `agent007_persona_show <name>` — show full details including system prompt and allowed tools
- `agent007_persona_switch <name>` — set the active persona for subsequent agent calls
- `agent007_agent_create` — create a new custom persona (action: `catalog` to browse archetypes, `save` to write the persona file)

---

### 6.10 Git Agent

agent007 includes a built-in git agent that performs common version control operations with awareness of the project's change impact.

#### Git Operations

- `agent007_git_status` — run `git status` and return structured output
- `agent007_git_diff` — return staged and unstaged diffs
- `agent007_git_log` — show the last N commits
- `agent007_git_commit` — create a commit with a provided message

#### Advanced Git Capabilities (CLI)

- Branch creation and checkout
- Pull request creation
- Impact analysis: given a diff, identify affected modules, tests, and documentation

---

### 6.11 Zones (Access Control)

Zones provide file-path-based access control, restricting which paths agent steps are allowed to read, write, or execute.

#### Zone Configuration

- Zones are configured in agent007's config file with a list of path patterns and permitted operations (`read`, `write`, `execute`) per zone
- Multiple zones can be defined to create layered access policies (e.g., "read-only for vendor/, read-write for src/")

#### Zone Enforcement

- `agent007_zone_check` — programmatically verify whether a given operation on a path is allowed
- Violations are surfaced as errors to the requesting tool or workflow step
- Zone checks can be incorporated into hooks for audit logging

---

### 6.12 Web Dashboard & TUI

agent007 provides two monitoring and management interfaces for users who prefer visual feedback.

#### Web Dashboard (port 8007)

- Served by an Axum HTTP server with WebSocket support for real-time updates
- Displays: active sessions, run history, skill invocations, workflow execution state, token usage, and memory contents
- Accessible at `http://localhost:8007` when `agent007 serve` is running

#### TUI (Terminal UI)

- A ratatui-based interactive terminal dashboard
- Available as an alternative to the web dashboard for terminal-only environments
- Displays the same core information: active runs, session history, and system health

---

### 6.13 MCP Server

The MCP (Model Context Protocol) server is the primary integration surface between agent007 and AI editors.

#### Transport

- **stdio:** The MCP server runs as a subprocess of the AI editor, communicating over stdin/stdout; this is the standard and default mode
- The editor invokes the agent007 binary with the `serve` subcommand; no persistent network port is required for core MCP operation

#### Tool Registration

- All 44 tools are registered with the MCP server at startup and are discoverable by the editor's AI assistant via the MCP tool listing protocol
- Tool descriptions include parameter schemas, enabling the AI to invoke tools correctly without explicit user instruction

#### Health & Config

- `agent007_health` returns memory directory status, loaded skill count, persona count, and zone configuration
- `agent007_config_show` returns the full current configuration as TOML

---

## 7. Technical Requirements

### Performance

| Requirement | Target |
|-------------|--------|
| MCP server startup time | < 500 ms |
| Skill render and dispatch latency (excluding model inference) | < 200 ms |
| Workflow step scheduling overhead | < 100 ms |
| Memory key-value read | < 50 ms |
| Web dashboard page load | < 1 second |

### Platform Compatibility

- **Operating systems:** macOS (primary), Linux (supported), Windows (not yet tested)
- **Architecture:** x86-64, ARM64 (Apple Silicon)
- **Rust MSRV:** Defined in `Cargo.toml`; follows latest stable with a reasonable lag

### Distribution

- Single statically-linked binary: `agent007`
- No external runtime dependencies required for core features (excluding optional Ollama for local models and LanceDB for vector search)
- Installable via `cargo install`, direct binary download, or package manager (planned)

### Configuration

- Primary config file: `~/.agent007/config.toml` (user-global) and `.agent007/config.toml` (project-local)
- Project config overrides user-global config for any key that is set in both
- All config is human-readable TOML; no binary config formats

### Security

- **Local-first:** No data leaves the machine except for model API calls to configured providers
- **Zone enforcement** prevents agent steps from reading or writing outside permitted paths
- **Hooks** execute shell commands with the full permissions of the running user; users are responsible for the security of hook scripts
- Secrets (API keys) are read from environment variables or a secrets file; they are never stored in memory artifacts or committed to skill/workflow files

---

## 8. Known Limitations & Future Work

The following gaps exist in the current production system. They are acknowledged and tracked for future resolution.

### Not Yet Implemented

| Gap | Description |
|-----|-------------|
| ModelRouter dispatch wiring | The router category-to-skill dispatch path is defined but not fully wired to skill execution; skills currently use the configured default model unless overridden explicitly |
| PromptOptimizer auto-trigger | `FeedbackEntry` records are accumulated but the optimizer does not yet trigger automatically at a threshold (e.g., every 20 entries); manual invocation is required |
| LanceDB vector search | The LanceDB integration is present but requires an embedding model to be configured before semantic memory search is active; falls back to exact key-value lookup |
| Evaluator/router workflow step types | The workflow engine spec includes `evaluator` and `router` step types for conditional branching and result scoring; these are not yet implemented; current DAGs are purely dependency-ordered |

### Known Constraints

| Constraint | Description |
|------------|-------------|
| Single-user, local-only | No multi-user or team server mode; team sharing requires committing skill/workflow files to source control |
| Windows support untested | The binary compiles for Windows but has not been validated on Windows environments |
| No streaming skill output | Skill execution returns the full response; token streaming within a skill run is not yet surfaced through the MCP protocol |
| MCP approval gates require editor support | Hosted-MCP approval gates work when the editor supports multi-turn MCP interactions; editors that batch all tool calls may not surface approval prompts correctly |

---

## 9. Glossary

| Term | Definition |
|------|------------|
| **MCP** | Model Context Protocol — an open protocol for AI editors to communicate with tool servers via structured JSON-RPC messages |
| **Skill** | A reusable, parameterized prompt artifact defined as a Markdown file with YAML frontmatter and a Tera template body; invoked by trigger (e.g., `/code-document`) |
| **Workflow** | A YAML-defined DAG of agent steps with dependency ordering, parallelism, and optional approval gates |
| **Persona** | A named agent identity with a system prompt, preferred model, and allowed tool set; used to give workflow steps consistent specialist behavior |
| **Hook** | A shell command registered to fire automatically on a named lifecycle event (e.g., `PostAgentRun`) |
| **Zone** | A file-path-based access control rule defining which paths an agent step may read, write, or execute |
| **Hosted-MCP** | An execution mode where the workflow engine runs inside the MCP server, and the host LLM drives step execution by submitting outputs through the MCP protocol |
| **Tera** | A Jinja2-compatible template engine for Rust, used to render skill and workflow step prompts |
| **LanceDB** | An embedded vector database used for semantic memory search |
| **RoutingConfig** | Configuration that maps task categories (e.g., `reasoning`, `fast_local`) to model providers and model names |
| **FeedbackEntry** | A recorded data point from a completed skill or workflow step, including token counts, model, and output; used by the learning system |
| **PromptOptimizer** | A component that analyzes FeedbackEntry data to generate improved prompt suggestions for frequently-used skills |
| **Repo Brain** | A persistent summary of the current repository's structure, patterns, and conventions, stored in project memory and refreshed on demand |
| **Session** | A single continuous invocation of agent007, encompassing all tool calls, skill runs, and workflow steps within that activation |
| **Trigger** | A slash-prefixed string (e.g., `/dev-debug`) that uniquely identifies a skill and is used to invoke it |
| **DAG** | Directed Acyclic Graph — the data structure used to represent workflow step dependencies; guarantees no circular dependencies |
