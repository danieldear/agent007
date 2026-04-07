# Workflow Orchestration Patterns & Visual Designer Enhancements

**Date**: 2026-03-26
**Status**: Draft → Awaiting Review

## Problem

The workflow visual designer is missing basic UX (node/edge deletion) and only supports simple DAG execution. Real-world agent orchestration requires loops (retry until quality passes) and conditional branching (route tasks to the right specialist). The engine has no mechanism for either.

Additionally, users have no starting-point templates — they must build every workflow from scratch, even for common patterns like fan-out or review loops.

## Goals

1. Add node and edge deletion to the visual workflow designer
2. Provide 5 pre-built workflow templates for common orchestration patterns
3. Extend the workflow engine with two new step types: `evaluator` (loop-back) and `router` (conditional branch)
4. Support hybrid evaluation: rule-based YAML conditions + LLM-driven structured decisions
5. Maintain full backward compatibility with existing workflows

## Non-Goals

- TUI changes (separate effort)
- Live execution monitoring in the web dashboard
- Workflow execution from the web UI (workflows run via CLI/MCP)
- Topology enforcement (users draw any graph; engine infers execution plan)

## Token Savings Rationale

These patterns exist to reduce token consumption:

- **Pipeline**: Each step gets only the previous output, not full conversation history. O(N) vs O(N²).
- **Fan-out**: N agents each process 1 unit of work (small context) vs 1 agent processing N units sequentially (growing context).
- **Router**: Routes simple tasks to cheap models, complex to expensive ones. 70% of subtasks handled by Haiku instead of Opus = 5-10x cost reduction.
- **Evaluator**: Retry re-sends only the failing step's context, not the entire conversation. Combined with memory, the coder reads why it failed and avoids repeating mistakes.
- **Hierarchical**: Coordinator does planning (small context), workers get only their slice.
- **Memory integration**: Agents read compressed knowledge from `.agent007/memory/` instead of re-reading source files. ~140 tokens vs ~20,000 tokens for project context.

Combined savings: 200-400x on token-heavy projects.

---

## Design

### 1. Visual Designer: Node & Edge Deletion

**Node deletion**:
- Right-click a node → context menu with "Delete Node"
- Select a node + press `Backspace` or `Delete` key
- Deleting a node removes all connected edges

**Edge deletion**:
- Click an edge to select it, then press `Backspace` or `Delete`
- Right-click an edge → context menu with "Delete Edge"

**Implementation**: Vue Flow supports `onNodesDelete` and `onEdgesDelete` events. Wire these to the existing reactive `nodes` and `edges` arrays in `WorkflowsView.vue`.

### 2. Workflow Templates

The "New" button becomes a dropdown with 6 options:

| Template | Shape | Use Case |
|---|---|---|
| Empty | Blank canvas | Custom workflows |
| Pipeline | A → B → C | Sequential phases (TDD, SPARC) |
| Fan-out / Fan-in | 1 → N parallel → 1 merge | Multi-file analysis, parallel review |
| Hierarchical | Coordinator → sub-agents → workers | Full-stack features, large refactors |
| Review Loop | Coder → Reviewer(evaluator) ↺ → Deployer | Quality-gated code, iterative refinement |
| Router | Triage(router) → specialist branches | Bug triage, complexity-based model routing |

Each template creates a pre-laid-out graph with agents, prompts, edges, and proper step types. Users customize from there.

**API**: `GET /api/workflow-templates` returns the list of templates with their full graph definitions (nodes, edges, step configs).

#### Template Definitions

**Pipeline**:
```yaml
name: pipeline-template
steps:
  - id: research
    agent: Researcher
    prompt: "Research best practices for: {{task}}"
    output: research_notes
  - id: design
    agent: Architect
    prompt: "Design based on: {{research_notes}}"
    output: plan
    depends_on: [research]
  - id: implement
    agent: Coder
    prompt: "Implement: {{plan}}"
    output: code
    depends_on: [design]
```

**Fan-out / Fan-in**:
```yaml
name: fan-out-template
steps:
  - id: split
    agent: Architect
    prompt: "Break down into independent concerns: {{task}}"
    output: concerns
  - id: security-review
    agent: SecurityReviewer
    prompt: "Security analysis: {{concerns}}"
    output: security_report
    depends_on: [split]
  - id: performance-review
    agent: PerformanceEngineer
    prompt: "Performance analysis: {{concerns}}"
    output: perf_report
    depends_on: [split]
  - id: style-review
    agent: CodeReviewer
    prompt: "Style and correctness review: {{concerns}}"
    output: style_report
    depends_on: [split]
  - id: merge
    agent: Architect
    prompt: "Synthesize all findings: {{security_report}} {{perf_report}} {{style_report}}"
    output: final_report
    depends_on: [security-review, performance-review, style-review]
```

**Hierarchical**:
```yaml
name: hierarchical-template
steps:
  - id: plan
    agent: Architect
    prompt: "Break this into frontend, backend, infra tasks: {{task}}"
    output: breakdown
  - id: frontend
    agent: UIUXDesigner
    prompt: "Implement UI components: {{breakdown}}"
    output: ui_code
    depends_on: [plan]
  - id: backend
    agent: Coder
    prompt: "Implement API layer: {{breakdown}}"
    output: api_code
    depends_on: [plan]
  - id: infra
    agent: DevOpsEngineer
    prompt: "Setup infrastructure: {{breakdown}}"
    output: infra_config
    depends_on: [plan]
  - id: integrate
    agent: Architect
    prompt: "Integration verification: {{ui_code}} {{api_code}} {{infra_config}}"
    output: integrated
    depends_on: [frontend, backend, infra]
```

**Review Loop**:
```yaml
name: review-loop-template
steps:
  - id: implement
    agent: Coder
    prompt: "Implement {{task}}. Previous feedback: {{review_result}}"
    output: code
  - id: review
    type: evaluator
    agent: CodeReviewer
    prompt: >
      Review this code for correctness and quality: {{code}}
      Respond with JSON: {"verdict": "pass" or "retry", "reason": "..."}
    output: review_result
    evaluate:
      decision_field: verdict
      on_pass: deploy
      on_fail: implement
      max_retries: 3
    depends_on: [implement]
  - id: deploy
    agent: DevOpsEngineer
    prompt: "Deploy verified code: {{code}}"
    output: deployment
    depends_on: [review]
```

**Router**:
```yaml
name: router-template
steps:
  - id: classify
    type: router
    agent: Researcher
    prompt: "Classify this task. Respond with one of: frontend, backend, infra. Task: {{task}}"
    output: classification
    routes:
      - when: "frontend"
        goto: ui-work
      - when: "backend"
        goto: api-work
      - goto: infra-work
        default: true
  - id: ui-work
    agent: UIUXDesigner
    prompt: "Handle frontend task: {{task}}"
    output: result
  - id: api-work
    agent: Coder
    prompt: "Handle backend task: {{task}}"
    output: result
  - id: infra-work
    agent: DevOpsEngineer
    prompt: "Handle infrastructure task: {{task}}"
    output: result
  - id: summarize
    agent: Researcher
    prompt: "Summarize the outcome: {{result}}"
    output: summary
    depends_on: [ui-work, api-work, infra-work]
```

### 3. New Step Types

#### 3a. Type Definitions (Rust)

Add to `crates/workflows/src/types.rs`:

```rust
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StepType {
    #[default]
    Execute,
    Evaluator,
    Router,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct EvaluateConfig {
    pub condition: Option<String>,
    pub decision_field: Option<String>,
    pub on_pass: String,
    pub on_fail: String,
    pub max_retries: Option<u32>,  // default: 3
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RouteConfig {
    pub when: Option<String>,
    pub goto: String,
    #[serde(default)]
    pub default: bool,
}
```

New fields on `StepDef`:

```rust
pub struct StepDef {
    // ... existing fields unchanged ...
    #[serde(default)]
    pub r#type: StepType,
    pub evaluate: Option<EvaluateConfig>,
    pub routes: Option<Vec<RouteConfig>>,
}
```

All new fields are `Option` or have `#[serde(default)]`, so existing YAML/TOML workflows deserialize without changes.

#### 3b. Evaluation Logic

**Rule-based** (`condition` field set):
- Simple string matching: `"{{output}} contains 'PASS'"`
- The engine performs template substitution on the condition string, then evaluates it
- Supported operators: `contains`, `equals`, `starts_with`, `not_contains`

**LLM-driven** (`decision_field` field set):
- The agent's output is parsed as JSON
- The value at `decision_field` is extracted
- `"pass"` → follow `on_pass`, anything else → follow `on_fail`

**Hybrid**: If both `condition` and `decision_field` are set, `condition` takes priority (rule-based is cheaper).

#### 3c. Routing Logic

The router step runs its agent, gets the output, then matches against `routes`:
1. Trim and lowercase the agent output
2. Check each route's `when` value for a substring match
3. If matched, activate that route's `goto` step; skip all other branches
4. If no match, use the route with `default: true`
5. If no default and no match, fail the workflow with a descriptive error

### 4. DAG Validator Changes

File: `crates/workflows/src/dag.rs`

The current validator rejects all cycles. With evaluator back-edges:

1. **Identify evaluator back-edges**: For each step with `type: evaluator`, the `on_fail` edge pointing to an earlier step is a back-edge
2. **Strip back-edges before toposort**: Remove evaluator back-edges from the petgraph before running `toposort()`. This prevents false cycle detection
3. **Re-attach as metadata**: After validation, return back-edges as separate metadata so the runner knows about them
4. **Validate router targets**: Every `goto` in a router's `routes` must reference a valid step ID
5. **Validate evaluator targets**: Both `on_pass` and `on_fail` must reference valid step IDs
6. **Require max_retries**: Every evaluator step must have `max_retries` set (engine defaults to 3 if absent)

Updated return type:

```rust
pub struct ValidatedDag {
    pub batches: Vec<Vec<String>>,
    pub back_edges: Vec<BackEdge>,
    pub router_branches: Vec<RouterBranch>,
}

pub struct BackEdge {
    pub evaluator_step: String,
    pub on_fail_target: String,
    pub max_retries: u32,
}

pub struct RouterBranch {
    pub router_step: String,
    pub routes: Vec<RouteConfig>,
}
```

### 5. Runner Changes

File: `crates/workflows/src/runner.rs`

Updated execution loop:

```
initialize retry_counts: HashMap<String, u32>

for batch in validated_dag.batches:
    determine which steps in this batch are active:
        - skip steps disabled by router (not on selected branch)
        - include steps re-queued by evaluator retry

    run active steps concurrently

    for each completed step:
        match step.type:
            Execute → collect output, proceed normally

            Evaluator →
                evaluate condition or parse decision_field
                if PASS:
                    mark on_pass step as ready
                if FAIL:
                    increment retry_counts[step.id]
                    if retry_counts[step.id] >= max_retries:
                        fail workflow with "max retries exceeded" error
                    else:
                        re-queue on_fail step for next iteration
                        inject current output into on_fail step's context

            Router →
                match output against routes
                mark selected goto step as active
                mark all other branch steps as skipped

    check budget limits (existing logic)
```

**Re-queue mechanism**: When an evaluator retries, the runner creates a mini-loop:
1. Re-execute the `on_fail` target step with the evaluator's feedback injected
2. Re-execute the evaluator step with the new output
3. Repeat until pass or max_retries

This loop happens within the runner, not by modifying the DAG structure.

### 6. Visual Designer: Node Type Rendering

**Normal (execute) nodes**: Current teal/green style, unchanged.

**Evaluator nodes**: Orange border (`border-orange-500`), loop icon (↺) in the header. Two output handles labeled "pass" (green) and "retry" (orange).

**Router nodes**: Purple border (`border-purple-500`), branch icon (⑂) in the header. Multiple output handles, one per route, each labeled with the `when` condition.

**Edge labels**: Edges from evaluator nodes show "pass" or "retry". Edges from router nodes show the condition value ("frontend", "backend", etc.).

New Vue components:
- `crates/web/frontend/src/components/EvaluatorNode.vue`
- `crates/web/frontend/src/components/RouterNode.vue`

Both follow the same pattern as the existing `AgentNode.vue`.

### 7. Updated WorkflowsView.vue

Changes to the existing view:

1. **Delete support**: Wire `onNodesDelete` and `onEdgesDelete` events. Add keyboard handler for Backspace/Delete.
2. **Template dropdown**: Replace the "New" button with a dropdown. On template select, call `GET /api/workflow-templates/{name}` and populate the canvas.
3. **Node type registration**: Register `EvaluatorNode` and `RouterNode` as custom Vue Flow node types alongside the existing `AgentNode`.
4. **Save/load**: The YAML serializer/deserializer handles `type`, `evaluate`, and `routes` fields. The graph → YAML and YAML → graph converters map evaluator/router properties to their respective node types and edge labels.
5. **Context menu**: Right-click on canvas background → "Add node" submenu (agent types + evaluator + router). Right-click on node → "Delete", "Edit". Right-click on edge → "Delete".

### 8. Web API Changes

New endpoint:

```
GET /api/workflow-templates
  → Returns: [{ name, description, steps: [...] }]

GET /api/workflow-templates/{name}
  → Returns: { name, description, steps: [...] }
```

Templates are hardcoded in the Rust backend (not user-editable). They serve as starting points.

Existing `POST /api/workflows` and `GET /api/workflows/{name}` handle the new step type fields transparently via serde.

---

## Files Changed

| Area | File | Change |
|---|---|---|
| Types | `crates/workflows/src/types.rs` | Add `StepType`, `EvaluateConfig`, `RouteConfig`; new fields on `StepDef` |
| DAG | `crates/workflows/src/dag.rs` | `ValidatedDag` return type; handle evaluator back-edges; validate router targets |
| Runner | `crates/workflows/src/runner.rs` | Evaluator retry loop; router branch selection; retry counter |
| Web API | `crates/web/src/api.rs` | Template list/get endpoints |
| Web Server | `crates/web/src/server.rs` | Register template routes |
| Vue: Workflows | `crates/web/frontend/src/views/WorkflowsView.vue` | Delete support, template dropdown, node type registration, context menu |
| Vue: EvaluatorNode | `crates/web/frontend/src/components/EvaluatorNode.vue` | New: orange evaluator node component |
| Vue: RouterNode | `crates/web/frontend/src/components/RouterNode.vue` | New: purple router node component |
| Vue: CSS | `crates/web/frontend/src/assets/main.css` | Styles for evaluator/router nodes |

## Testing

- **Types**: Deserialize existing YAML without new fields → unchanged behavior (backward compat)
- **Types**: Deserialize YAML with `type: evaluator` and `evaluate` config → correct parsing
- **Types**: Deserialize YAML with `type: router` and `routes` config → correct parsing
- **DAG**: Evaluator back-edge is not flagged as cycle
- **DAG**: Actual structural cycle (non-evaluator) is still detected
- **DAG**: Router `goto` referencing non-existent step → error
- **DAG**: Missing `max_retries` defaults to 3
- **Runner**: Evaluator pass → proceeds to on_pass step
- **Runner**: Evaluator fail → retries on_fail step up to max_retries
- **Runner**: Evaluator exceeds max_retries → workflow fails with descriptive error
- **Runner**: Router matches correct branch → only that branch executes
- **Runner**: Router no match + no default → workflow fails with error
- **Runner**: Router no match + default → default branch executes
- **Web**: `GET /api/workflow-templates` returns 5 templates
- **Vue**: Nodes can be deleted via right-click and keyboard
- **Vue**: Edges can be deleted via right-click and keyboard
- **Vue**: Template selection populates canvas with correct graph
- **Vue**: Evaluator/router nodes render with distinct styles
