# agent007 — Workflow Engine Design

*Deep-dive design document for the workflow subsystem. Current-state only.*

---

## 1. Overview

The workflow engine lets a caller define a multi-step, multi-agent pipeline in a YAML file.
Steps have explicit data dependencies (`depends_on`, `inputs`); the engine resolves them into
a directed acyclic graph (DAG), runs independent steps in parallel, threads outputs as
template variables into downstream prompts, and exposes an approval gate that pauses
execution for human review.

Two execution modes exist:

| Mode | Driver | Entry point |
|------|--------|-------------|
| **Autonomous** | `WorkflowRunner` calls LLM directly | `WorkflowRunner::run()` |
| **Hosted-MCP** | Host LLM drives step-by-step via MCP tools | `HostedWorkflowEngine` + `workflow_start / next / submit_step` MCP tools |

The Hosted-MCP mode is how editor integrations use workflows today. The engine emits ready
steps as structured JSON; the host LLM executes the prompt and submits the output back.

---

## 2. WorkflowDef YAML Schema

Workflow files live in `~/.agent007/workflows/*.yaml` (or project-local
`.agent007/workflows/*.yaml`). The format is YAML but parsed as TOML in `WorkflowLoader`
via `serde_yaml` — the two formats are structurally equivalent for the subset agent007 uses.

### Top-level fields

```yaml
name: string          # required; used as the workflow identifier
description: string   # optional; shown in listings
budget:               # optional budget enforcement
  max_tokens_per_session: 500000
  max_usd_per_task: 2.00
  alert_at_percent: 80
  on_exceed: "pause"  # "pause" | "stop" | "alert-only"
steps:
  - ...               # array of StepDef
```

### StepDef fields

```yaml
id: string            # required; unique within the workflow
agent: string         # required; persona name (matched via PersonaProvider)
type: execute         # execute (default) | evaluator | router | sub-workflow
model: string         # optional; overrides persona's preferred_model
prompt: string        # Tera template; required for execute steps unless skill is set
skill: string         # alternative to prompt: invoke a named skill trigger
output: string        # variable name for this step's output (used by downstream steps)
inputs:               # list of output variable names this step reads
  - string
depends_on:           # explicit ordering when there is no data dependency
  - string
requires_approval: bool  # if true, pause before this step's output is accepted

# For type: evaluator
evaluate:
  condition: string        # optional Tera expression evaluated against outputs
  decision_field: string   # output variable name containing pass/fail verdict
  on_pass: step_id         # jump to this step on pass
  on_fail: step_id         # jump back to this step on fail (back-edge)
  max_retries: 3           # default 3

# For type: router
routes:
  - when: string           # optional condition string
    goto: step_id          # target step
    default: bool          # true = fallback route

# For type: sub-workflow
workflow: string           # workflow name (without .yaml extension)
```

### Built-in template variables

These are injected at runtime without needing an `inputs` declaration:

| Variable | Source |
|----------|--------|
| `{{task}}` | The task string passed to `workflow_start` or `WorkflowRunner::run` |
| `{{memory.project}}` | Contents of the `project` memory scope |
| `{{memory.user}}` | Contents of the `user` memory scope |
| `{{memory.global}}` | Contents of the `global` memory scope |
| `{{memory.repo_brain}}` | The repo brain summary (if computed) |
| `{{rag_context}}` | RAG retrieval result for the current task |

---

## 3. Dependency Graph Execution

### 3.1 DAG construction (`crates/workflows/src/dag.rs`)

`DagValidator` builds a `petgraph::DiGraph<String, ()>` from the workflow definition:

1. **Schema validation** (`WorkflowDef::validate_schema`) — runs first, catches missing
   required fields and type-specific constraints before any graph work.

2. **Node creation** — one graph node per step, keyed by `step.id`.

3. **Edge creation** — two sources of edges:
   - **Data dependency**: if step B lists `"out_a"` in its `inputs`, and step A declares
     `output: out_a`, an edge `A → B` is added. This is the primary dependency mechanism.
   - **Explicit ordering**: `depends_on` adds edges without requiring a data output.

4. **Back-edge exemption** — evaluator `on_fail` targets create a back-edge (cycle) in the
   logical graph. `DagValidator` records these as `BackEdge` structs and explicitly skips
   them when adding edges to the petgraph, preventing false cycle detection.

5. **Cycle detection** — `petgraph::algo::toposort` is called on the forward-only graph.
   A `WorkflowError::CycleDetected` is returned if it fails.

6. **Level assignment** — each node's level = 1 + max(predecessor levels). Nodes with no
   predecessors are level 0.

7. **Batch extraction** — nodes at the same level form a batch; batches within a level run
   in parallel.

```
Example: code-review workflow

  security-review ──┐
  performance-review ┼──► synthesize
  quality-review ───┘

  Batch 0: [security-review, performance-review, quality-review]  (parallel)
  Batch 1: [synthesize]
```

### 3.2 Parallel execution in `WorkflowRunner`

For each batch, `WorkflowRunner` issues concurrent LLM calls using `tokio::join_all` (or
equivalent). Step outputs are collected and inserted into a `HashMap<String, String>` that
is passed as template context to all subsequent steps.

`WorkflowStepStatus` tracks each step through:
`Pending → Running → AwaitingApproval → Completed | Skipped | Failed`

### 3.3 `ValidatedDag` output

```rust
pub struct ValidatedDag {
    pub batches: Vec<Vec<String>>,       // topological batches for parallel execution
    pub back_edges: Vec<BackEdge>,       // evaluator retry edges
    pub router_branches: Vec<RouterBranch>, // router conditional branches
}
```

---

## 4. Hosted-MCP Execution Mode

In Hosted-MCP mode the host LLM (running in the editor) drives step execution. The engine
acts as a stateful coordinator; the host is responsible for generating text.

### 4.1 Protocol (three MCP tools)

```
agent007_workflow_start(name, task)
  └── Creates a new WorkflowRunState, persists to sessions/<run-id>/
      Returns: { session, status, ready_steps, ... }

agent007_workflow_next(session)
  └── Returns the next batch of ready HostedWorkflowStep objects (leased)
      Host LLM executes each step's prompt and calls submit_step

agent007_workflow_submit_step(session, step, output)
  └── Records step output into state.outputs
      Advances step status to Completed
      Returns updated HostedWorkflowProgress
```

### 4.2 `HostedWorkflowStep` — what the host receives

```rust
pub struct HostedWorkflowStep {
    pub id: String,
    pub agent: String,
    pub model_hint: String,          // from persona or step.model override
    pub system_prompt: Option<String>, // from PersonaSpec
    pub prompt: String,              // fully rendered Tera template
    pub output_key: Option<String>,  // variable name to store result under
    pub inputs: HashMap<String, String>, // resolved input values
    pub depends_on: Vec<String>,
    pub step_type: StepType,
    pub requires_approval: bool,
}
```

The `prompt` field is already rendered — the host LLM just executes it.

### 4.3 State machine

```
                  ┌─────────────────────────────┐
                  │  WorkflowRunStatus           │
                  │                             │
  start()      ─► │  Running                    │
                  │    │                        │
                  │    ├─ step requires_approval │
                  │    │    └─► WaitingApproval  │
                  │    │          │              │
                  │    │    approve/deny/edit    │
                  │    │          │              │
                  │    │          └─► Running    │
                  │    │                        │
                  │    ├─ all steps done ──────► │  Succeeded  │
                  │    │                        │
                  │    └─ step fails ──────────► │  Failed     │
                  └─────────────────────────────┘
```

### 4.4 Session persistence

`WorkflowRunState` is serialized to two JSON files on every mutation:

```
~/.agent007/sessions/<run-id>/
  workflow-request.json   ← { workflow, task }
  workflow-state.json     ← full WorkflowRunState
```

`agent007_workflow_resume(session)` reads these files and reconstructs the engine state,
allowing a workflow to survive CLI restarts.

---

## 5. Session State and Template Variable Flow

```
workflow_start("task description")
  │
  ▼
WorkflowRunState {
  workflow: "sparc",
  task: "task description",
  outputs: {}               ← grows as steps complete
  steps: [ { id: "spec", status: Pending }, ... ]
}

After step "spec" completes with output "specification":
  outputs: { "specification": "..." }

Step "pseudocode" receives rendered prompt:
  "Based on this specification:\n{{ specification }}\n..."
  └── Tera renders {{specification}} from outputs map

After step "pseudocode" completes:
  outputs: { "specification": "...", "pseudocode": "..." }
```

`render_prompt()` in `runner.rs` builds a Tera context from:
1. The accumulated `outputs` map (step outputs)
2. Built-in variables (`task`, `memory.*`, `rag_context`)
3. The step's explicit `inputs` list (validated subset of outputs)

If a referenced variable is missing at render time, `WorkflowError::StepFailed` is
returned with the template rendering error.

---

## 6. Approval Gate Mechanism

Steps with `requires_approval: true` pause execution after their output is produced but
before it is committed to the outputs map.

### 6.1 Autonomous mode (`ApprovalGate`)

Writes to stderr and reads from stdin interactively:

```
[APPROVAL REQUIRED] Step: architect
Output:
<step output here>

Approve? [y/n/edit]:
```

Responses: `y`/`yes` → Approve, `n`/`no` → Deny, `e`/`edit` → prompt for replacement content.

### 6.2 Hosted-MCP mode

The engine sets `state.pending_approval = Some(PendingApproval { ... })` and returns
`HostedWorkflowProgressStatus::AwaitingApproval`. The host calls:

```
agent007_workflow_approve(session, decision, [content])
  decision: "approve" | "deny" | "edit"
  content:  replacement text (required for "edit")
```

`ApprovalDecision` is stored in `state.approval_decisions` keyed by step ID.
`finalize_approved_steps()` is called at the start of every `dispatch` / `submit_step`
call to drain pending approvals and advance step status.

### 6.3 Approval outcomes

| Decision | Effect |
|----------|--------|
| `approve` | Step's output is stored; workflow continues |
| `deny` | Step is marked `Failed`; workflow status → `Failed` |
| `edit` | Provided content replaces the step's output; workflow continues |

---

## 7. Built-in Workflow Templates

All seven templates live in `~/.agent007/workflows/`. They are YAML files loaded at
runtime; they are not compiled into the binary.

### 7.1 `tdd.yaml` — TDD Red-Green-Refactor

**3 sequential steps.** Simple linear chain.

| Step | Agent | Reads | Writes |
|------|-------|-------|--------|
| `red` | TestDesigner | `task`, `memory.repo_brain` | `failing_tests` |
| `green` | Coder | `failing_tests` | `implementation` |
| `blue` | ExpertCoder | `implementation`, `failing_tests` | `refactored_code` |

### 7.2 `sparc.yaml` — SPARC Methodology

**5 sequential steps.** Each phase feeds the next.

| Step | Agent | Phase |
|------|-------|-------|
| `spec` | Researcher | Specification |
| `pseudocode` | Coder | Pseudocode |
| `architecture` | Architect | Architecture |
| `refinement` | CodeReviewer | Refinement |
| `completion` | ExpertCoder | Completion |

### 7.3 `code-review.yaml` — Parallel Review Team

**4 steps; first 3 run in parallel.**

```
Batch 0: security-review ─┐
         performance-review ┼──► Batch 1: synthesize
         quality-review ───┘
```

Agents: SecurityReviewer, PerformanceEngineer, CodeReviewer (×2).

### 7.4 `log-analysis.yaml` — Parallel Log Analysis

**4 steps; first 3 run in parallel.**

```
Batch 0: find-errors ──┐
         find-patterns ─┼──► Batch 1: synthesize
         find-security ─┘
```

### 7.5 `security-audit.yaml` — Deep Security Audit

**4 steps; first 4 run in parallel** (OWASP, secrets, threat model, dependencies)
**→ 1 synthesis step.**

```
Batch 0: owasp-scan ──────┐
         secrets-scan ─────┼──► Batch 1: synthesize
         threat-model ─────┤
         dependency-scan ──┘
```

### 7.6 `ideation.yaml` — Ideation-to-Plan Pipeline

**8 steps; sequential with a human approval gate after research.**

```
research → [APPROVAL] → document-ideation → prd →
architecture → document-design → project-plan → document-milestones
```

Notable: the `architecture` step reads the PRD output, enforcing that requirements
drive design rather than design driving requirements.

### 7.7 `feature.yaml` — Full-Cycle Feature Delivery

**14 steps; the most complex built-in workflow.**

```
load-context ──► research ──► feature-spec ──► architecture ──► implementation
  ──► [APPROVAL] ──► {parallel: code-review, security-review, performance-review,
                              gap-analysis, issues-review}
  ──► rework ──► test-design ──► test-coverage-review ──► documentation
  ──► [APPROVAL: release-signoff]
```

Steps use per-step `model` overrides (e.g., `claude-haiku-4-5-20251001` for context-loading
steps, faster and cheaper).

---

## 8. Known Gaps

### 8.1 Evaluator and Router step types — schema only

`StepType::Evaluator` and `StepType::Router` are fully defined in `types.rs` and validated
by `DagValidator` (including back-edge exemption for evaluators and `goto` target
resolution for routers). However:

- `WorkflowRunner::run()` does not yet branch on `step.r#type`. Evaluator steps are
  executed as regular execute steps; their `evaluate.on_fail` back-edge is never triggered.
- `HostedWorkflowEngine` similarly does not conditionally skip or jump based on
  `evaluate.decision_field` or `routes`.
- `SubWorkflow` step type is defined and schema-validated but not executed.

All three are forward-declared in the type system to avoid breaking YAML compatibility
when the execution logic is added.

### 8.2 No streaming step outputs

`WorkflowRunner` and `HostedWorkflowEngine` collect the full LLM response before
advancing. Long-running steps produce no incremental output. This is a UX limitation
for the web dashboard.

### 8.3 Budget enforcement — alert only

`BudgetConfig` is deserialized and `BudgetUsed` is tracked (tokens accumulated per
step via `estimate_tokens()`), but the `on_exceed` action (`"pause"` / `"stop"`) is
not yet enforced at runtime — `check_budget()` logs a warning but does not halt execution.

### 8.4 `WorkflowRunState` write is not atomic

`workflow-state.json` is written with `std::fs::write` (truncate + write). A crash
between truncate and write completion would leave a corrupt file. A write-then-rename
pattern would make this safe.
