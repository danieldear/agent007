# Architecture

agent007 is a layered Rust workspace of 22 crates. The CLI binary (`crates/cli`) is the only binary; all other crates are libraries.

---

## Layer overview

```mermaid
graph TD
    subgraph Clients["Clients"]
        E1[Claude Code]
        E2[Cursor / Codex]
        E3[Copilot / Zed]
        WEB[Web Browser]
        TERM[Terminal]
    end

    subgraph Entry["Entry layer — crates/cli"]
        MCP["MCP server\n(stdio)"]
        HTTP["HTTP + WebSocket\n:8007"]
        CLI["clap CLI\nrun / serve / skill / workflow"]
    end

    subgraph Core["Orchestration layer"]
        ORCH[core::Orchestrator]
        DISP[core::Dispatcher]
        WF[workflows::WorkflowEngine]
        SK[skills::SkillDispatcher]
        ROUTER[models::ModelRouter]
    end

    subgraph Storage["Storage layer"]
        MEM[memory::MemoryStore\nKV + LanceDB vector]
        LEARN[learning::LearningStore]
        RUNS[sessions/ run history]
    end

    subgraph Infra["Infrastructure"]
        HOOKS[hooks::HookExecutor]
        ZONES[zones — path ACL]
        GIT[git-agent]
        P2P[p2p::P2pService]
        SHARE[sharing::BundleAsset]
    end

    E1 & E2 & E3 -->|MCP stdio| MCP
    WEB -->|HTTP/WS| HTTP
    TERM --> CLI

    MCP & CLI --> ORCH
    HTTP --> WF
    HTTP --> SK

    ORCH --> DISP
    DISP --> ROUTER
    DISP --> WF
    DISP --> SK
    ROUTER -->|LLM API| LLM[(Claude / Codex\nOllama)]

    WF --> HOOKS
    SK --> HOOKS
    SK --> MEM
    ORCH --> MEM
    ORCH --> LEARN
    LEARN --> MEM
    ORCH --> RUNS
    WF --> RUNS

    HTTP --> SHARE
    P2P --> SHARE
```

---

## Crate dependency graph

```mermaid
graph LR
    core --> models
    core --> memory
    core --> hooks
    core --> personas
    core --> learning
    core --> zones
    core --> sharing

    memory --> skills
    models --> skills
    personas --> skills
    hooks --> skills

    skills --> workflows
    memory --> workflows
    models --> workflows
    hooks --> workflows
    learning --> workflows
    personas --> workflows

    core --> mcp
    core --> git-agent
    core --> ide-bridge
    core --> lsp-client
    core --> tui
    core --> custom-agents
    core --> simulation
    core --> testing
    core --> extensions

    workflows --> web
    memory --> web
    models --> web
    sharing --> web
    learning --> web
    extensions --> web

    sharing --> p2p
    core --> p2p

    style core fill:#1a1a2e,color:#e0e0e0
    style cli fill:#16213e,color:#e0e0e0
```

> `crates/cli` depends on every crate above and is the binary assembly point.
> `crates/core` is the dependency root — it defines `Task`, `AgentId`, `Dispatcher`, `Budget`, and the shared context types that all other crates consume.

---

## MCP tool call dispatch

```mermaid
flowchart TD
    CALL["call_tool(name, params)"]

    CALL --> R1{tool name}

    R1 -->|agent007_run| ORCH[core::Orchestrator\n→ Dispatcher\n→ ModelRouter\n→ LLM]
    R1 -->|agent007_skill_run| SKILL[skills::SkillDispatcher\n→ render Tera template\n→ LLM call]
    R1 -->|agent007_workflow_*| WFE[workflows::WorkflowEngine\n→ eval gate\n→ reliability engine\n→ hosted engine]
    R1 -->|agent007_memory_*| MEMS[memory::MemoryStore\n→ KV scope / vector]
    R1 -->|agent007_record_tokens| LS[learning::LearningStore\n→ FeedbackEntry\n→ InsightGenerator]
    R1 -->|agent007_mcp_tool_call| MCPC[mcp::McpClient\n→ downstream MCP server]
    R1 -->|bundle_export/import| BND[sharing::BundleAsset\nSHA-256 verified pack]

    SKILL -->|fire| HK1[hooks: OnSkillExecute]
    MEMS -->|fire| HK2[hooks: OnMemoryWrite]
    LS -->|fire| HK3[hooks: PostTaskComplete]
    LS --> IG[InsightGenerator\nwrites procedural memory\nwhen failure rate > threshold]

    ORCH --> TR[retrieval telemetry artifact\n→ persisted to run record]
    SKILL --> TR
```

---

## Workflow run lifecycle

```mermaid
stateDiagram-v2
    [*] --> Pending: workflow_start / workflow_run

    Pending --> Running: engine picks up run

    Running --> AwaitingApproval: approval gate reached
    AwaitingApproval --> Running: workflow_approve

    Running --> Recovering: step fails\n(reliability engine)
    Recovering --> Running: retry within budget
    Recovering --> Failed: max retries exhausted

    Running --> Complete: all steps done\neval gate = pass/warn

    Running --> Blocked: eval gate = block

    Complete --> [*]
    Failed --> [*]
    Blocked --> [*]

    note right of Recovering
        Budget governor checks spend
        before each retry. If over
        threshold → graceful degradation
        → model downgrade → abort.
    end note

    note right of AwaitingApproval
        Approval ownership rule:
        dashboard-initiated → dashboard owns it.
        MCP-initiated → calling client owns it.
    end note
```

---

## Hosted workflow session protocol

The hosted engine lets a MCP-connected LLM (Claude Code, Cursor, etc.) execute each workflow step itself, step-by-step, via a session loop.

```mermaid
sequenceDiagram
    participant Editor as AI Editor
    participant CLI as crates/cli (MCP)
    participant WF as workflows::HostedEngine
    participant LLM as LLM API

    Editor->>CLI: agent007_workflow_start(name, args)
    CLI->>WF: create_session(name, args)
    WF-->>CLI: {session_id, step_prompt, step_index}
    CLI-->>Editor: {session_id, step_prompt}

    loop each step
        Editor->>LLM: run step_prompt
        LLM-->>Editor: step_output
        Editor->>CLI: agent007_workflow_submit_step(session_id, output)
        CLI->>WF: submit_step(session_id, output)

        alt approval gate
            WF-->>CLI: {status: awaiting_approval, rationale}
            CLI-->>Editor: awaiting approval
            Editor->>CLI: agent007_workflow_approve(session_id)
            CLI->>WF: approve(session_id)
        end

        WF-->>CLI: {next_step_prompt} or {status: complete}
        CLI-->>Editor: next prompt or done
    end

    Editor->>CLI: agent007_record_tokens(run_id, ...)
    CLI->>WF: finish_run(run_id, tokens)
```

---

## Memory and RAG context assembly

```mermaid
flowchart LR
    subgraph Warmup["Startup warmup (bounded)"]
        FILES["project files\n(capped by file count\n+ size limits)"]
        IDX["LanceDB indexer"]
        FILES --> IDX
    end

    subgraph Write["Write paths"]
        MW["agent007_memory_write\n(explicit KV)"]
        IG2["InsightGenerator\n(auto procedural memory\non failure patterns)"]
    end

    subgraph Store["memory::MemoryStore"]
        KV["KV scopes\nglobal / project / user / learning"]
        VEC["LanceDB vector index"]
    end

    subgraph Read["Context assembly at skill/run time"]
        QUERY["retrieval query\n(task description)"]
        HITS["vector hits + KV hits"]
        CTX["Tera context\n{{memory.project}}\n{{rag_context}}"]
        TEL["retrieval telemetry artifact\n(hit rate, fallback, mock flag)"]
    end

    IDX --> VEC
    MW --> KV
    IG2 --> KV

    QUERY --> VEC
    QUERY --> KV
    VEC --> HITS
    KV --> HITS
    HITS --> CTX
    HITS --> TEL
```

`AGENT007_RAG_WARMUP=0` disables startup indexing. Graceful degradation applies when the LanceDB path is unavailable — retrieval falls back to KV-only.

---

## Model routing and adaptive shadow

```mermaid
flowchart TD
    TASK["Task\n(type, context, budget)"]

    TASK --> ROUTER["models::ModelRouter\n(rule-based heuristics)"]
    TASK --> SHADOW["AdaptiveShadow\n(parallel, read-only)"]

    ROUTER -->|selected model| EXEC["LLM execution"]
    ROUTER -->|route decision| LOG1["route log\n→ run record"]

    SHADOW -->|recommended model| LOG2["shadow recommendation\n→ run record\n(not applied to execution)"]

    EXEC --> RESP["CompletionResponse\n(output + token counts)"]

    subgraph Dashboard["web dashboard — run detail"]
        RLOG["actual route used"]
        SLOG["routing recommendations\n(from shadow)"]
    end

    LOG1 --> RLOG
    LOG2 --> SLOG
```

Routing rules use: task type tag, budget pressure, historical success scores from `learning::LearningStore`. The shadow never changes execution — it only accumulates advisory recommendations for human review and eventual policy tuning.

---

## Skill execution path

```mermaid
flowchart TD
    TRIGGER["trigger: /my-skill args"]

    TRIGGER --> POLICY["personas::tool_policy_check\n(allowed_tools list)"]

    POLICY -->|strict mode: blocked| WARN["policy violation artifact\n→ execution halted"]
    POLICY -->|warn mode / allowed| LOAD["skills::SkillDispatcher\nload frontmatter + template"]

    LOAD --> SCOPE["resolve scope\nproject-local overrides global"]
    SCOPE --> MEM2["inject memory context\n{{memory.project}}, {{rag_context}}"]
    MEM2 --> TERA["render Tera template\n(args, memory, rag_context)"]

    TERA --> HOOK["fire hooks::OnSkillExecute\nHOOK_SKILL, HOOK_ARGS env vars"]
    HOOK --> LLM2["LLM call via ModelProvider\n(model from frontmatter or config default)"]

    LLM2 --> OUT["skill output"]
    LLM2 --> TEL2["retrieval telemetry artifact\n(hit_rate, vector_hits, fallback_hits)"]
    LLM2 --> FEED["learning::FeedbackEntry\n(outcome, reward, tokens)"]

    FEED --> INSIGHT["InsightGenerator check\nevery N feedbacks:\nfailure rate > threshold?\n→ write procedural memory"]
```

---

## P2P collaboration and sharing

```mermaid
sequenceDiagram
    participant UA as Peer A (agent007)
    participant UB as Peer B (agent007)
    participant DISC as mDNS discovery

    UA->>DISC: advertise(peer_id, addr, capabilities)
    UB->>DISC: advertise(peer_id, addr, capabilities)
    DISC-->>UA: discovered: Peer B
    DISC-->>UB: discovered: Peer A

    UA->>UA: sharing::BundleAsset.pack(skills/workflows)
    UA->>UA: p2p::identity::sign(envelope)
    UA->>UB: send CollaborationEnvelope

    UB->>UB: p2p::service::ingest_envelope
    UB->>UB: verify signature
    UB->>UB: policy::filter (allowed artifact classes)

    alt signature valid + policy allows
        UB->>UB: apply bundle (import skills/workflows)
        UB-->>UA: ack
    else tamper / replay / policy reject
        UB-->>UA: reject (reason)
    end
```

Sharing is opt-in and policy-gated. Unknown peers are rejected at ingest. Replay protection is enforced in `P2pService::ingest_envelope`. Raw prompts and outputs are excluded from bundles by default.

---

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
Loads skills from `~/.agent007/skills/` (global) and `.agent007/skills/` (project-local), scanning recursively to support skill folders. Project skills override global skills with the same trigger. Each skill is a `SkillFrontmatter` + template body rendered with Tera.

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

---

## Directory layout (~/.agent007/)

```
~/.agent007/
├── config.toml          Main configuration
├── skills/              User-defined skills (*.md, nested folders supported)
├── personas/            User-defined personas (*.md)
├── workflows/           User-defined workflows (*.yaml)
├── hooks/
│   └── hooks.toml       Hook configuration
├── memory/
│   ├── global/          Global key-value
│   ├── project/         Project-scoped key-value
│   ├── user/            User-scoped key-value
│   └── learning/        FeedbackEntry records
├── sessions/            Run history (JSON per run)
└── ports.toml           Per-project port registry
```
