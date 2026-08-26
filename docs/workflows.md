# Workflows

Workflows are multi-step agent pipelines defined as YAML files in `~/.agent007/workflows/`. Steps without `depends_on` run in parallel; steps with `depends_on` run after their predecessors complete.

## Running a workflow

**Command-style dispatch (Codex-friendly):**
```
agent007_dispatch command="$agent007 wf tdd Add rate limiting to the API"
```

**Via MCP (from your AI editor):**
```
agent007_workflow_run name="tdd" task="Add rate limiting to the API"
```

**Via CLI:**
```bash
agent007 workflow run tdd "Add rate limiting to the API"
```

`agent007_dispatch` is additive convenience. Direct workflow tools still work.

## Choose by intent, then depth

The catalog groups workflows by user intent. Some workflows share an intent but
apply a different level of ceremony; they are profiles, not exact duplicates.

| Intent | Lightweight | Standard/specialized | Full-cycle |
|---|---|---|---|
| Discover | `brainstorm` | - | `ideation` |
| Deliver | - | `tdd`, `sparc` | `feature` |
| Review | `code-review` | - | `security-audit` for deep security assurance |
| Diagnose | `log-analysis` | focused debugging skills | - |
| Deliberate | - | - | `llm-council` |

Use `tdd` for a bounded behavior with tests first, `sparc` for greenfield design,
and `feature` when production delivery needs approvals, parallel reviews,
documentation, and release sign-off. See [Catalog Architecture](./catalog-architecture.md)
for the long-term intent/profile model and optional domain-pack boundary.

## Hosted-MCP mode

For multi-step workflows where the host LLM executes each step:

```
agent007_workflow_start name="feature" task="..."
# Returns session ID

agent007_workflow_next session="<id>"
# Returns step prompts to execute

agent007_workflow_submit_step session="<id>" step="research" output="..."
# Submit your output; get next steps

agent007_workflow_approve session="<id>"
# Approve a human-gate step
```

### Approval ownership and dashboard resume

Approval-gated workflows now follow a split ownership model:

```text
workflow starts from Codex / Claude / Cursor / Copilot / Zed
-> approval surfaces back to the initiating client
-> the web dashboard stays read-only for that externally initiated run

workflow starts from the dashboard's standalone runtime
-> dashboard can show Resume Workflow
-> the standalone run can resume there
```

The practical rule is:
- externally initiated workflows are approved and continued in the initiating client
- dashboard-owned standalone workflows can still be resumed from the dashboard

For current limitations and guidance, see [Known Issues](./known-issues.md).

## Available workflows

### `tdd` — Test-driven development
**Steps:** 3 (sequential) | **Use when:** Writing a new feature or requirement with tests first.

```
red → green → refactor
```

1. **red** — Write failing tests that define the expected behavior
2. **green** — Write minimal implementation to make tests pass
3. **refactor** — Clean up code while keeping tests green

---

### `code-review` — Parallel code review
**Steps:** 4 (3 parallel + synthesize) | **Use when:** Reviewing code for quality, security, and performance.

```
security-review ─┐
performance-review ─┤─► synthesize
style-review ────┘
```

1. **security-review** — OWASP vulnerabilities, auth, injection, secrets
2. **performance-review** — Allocations, N+1 queries, blocking calls, caching
3. **style-review** — Code clarity, naming, patterns, maintainability
4. **synthesize** — Aggregates all findings into a severity-ranked report

---

### `sparc` — Full feature delivery (SPARC methodology)
**Steps:** 5 (sequential) | **Use when:** Building a new feature end-to-end from requirements.

```
spec → pseudocode → architecture → refinement → completion
```

1. **spec** — Detailed specification with requirements and constraints
2. **pseudocode** — Structured pseudocode with logic flow
3. **architecture** — Component design, data flow, interfaces
4. **refinement** — Review for correctness, edge cases, quality
5. **completion** — Final docs, tests, polish

---

### `log-analysis` — Parallel log analysis
**Steps:** 4 (3 parallel + synthesize) | **Use when:** Diagnosing errors, performance issues, or security events in logs.

```
error-finder ──┐
pattern-analyst ─┤─► synthesize
security-checker ┘
```

1. **error-finder** — Identify errors, exceptions, and crashes
2. **pattern-analyst** — Frequency patterns, timing, correlations
3. **security-checker** — Auth failures, anomalies, potential attacks
4. **synthesize** — Aggregated report with root cause and action items

---

### `feature` — Full-cycle delivery pipeline
**Steps:** 17 | **Use when:** Delivering a production feature with full review gates.

```
load-context → research → document-brief → feature-spec → architecture
    → implement → [APPROVAL GATE]
    → code-review, security-review, performance-review, gap-analysis, issue-analysis (parallel)
    → rework → test-design → test-coverage-review → document-feature
    → [APPROVAL GATE: release-signoff]
```

Human approval gates at `implement` and `release-signoff`.

---

### `ideation` — Idea to project plan
**Steps:** 7 | **Use when:** Turning a vague idea into a concrete PRD and architecture.

```
research → [APPROVAL] → document-ideation → prd → architecture → document-design → project-plan
```

---

### `security-audit` — Deep security audit
**Steps:** 5 (4 parallel + synthesize) | **Use when:** Full security review before a release.

```
owasp-scan ────────┐
secrets-scan ──────┤─► synthesize
threat-model ──────┤
dependency-scan ───┘
```

---

### `brainstorm` — Lightweight ideation pipeline
**Use when:** You need fast idea exploration with an approval checkpoint before committing to full architecture work.

Typical flow:
```
brainstorm -> approval -> PRD + ideation doc
```

---

### `llm-council` — Deep peer deliberation
**Use when:** An ambiguous or high-impact question benefits from independent
perspectives, direct challenges, revised positions, and explicit dissent.

The council frames the question, gathers parallel specialist views, routes
cross-examination questions, asks each member to revise its position, and produces
a decision memo with consensus, dissent, risks, evidence gaps, and next actions.
It is intentionally heavier than `brainstorm` and remains advisory.

---


## Eval Gates

Eval Gates score each workflow run against a rolling baseline and make a `pass / warn / block` decision before the run proceeds past the gate.

```yaml
# Configured per-workflow in the workflow YAML
eval_gate:
  min_baseline_runs: 5      # runs needed before gating starts
  baseline_window: 20       # rolling window size
  warn_threshold: 0.15      # score drop that triggers warn
  block_threshold: 0.30     # score drop that triggers block
```

**Decision fields surfaced in the dashboard:**

| Field | Description |
|---|---|
| `decision` | `pass` / `warn` / `block` |
| `baseline_sample_size` | Runs in the current baseline window |
| `min_baseline_runs` | Minimum runs needed to activate the gate |
| `baseline_window` | Rolling window size used for scoring |
| `reason_codes` | Machine-readable codes explaining the decision |
| `message` | Human-readable explanation |

When the gate fires `block`, the workflow step is halted and the run is marked failed. `warn` continues but records the degradation. `pass` continues silently.

Eval gate results are visible in the **Persisted Runs** accordion in the web dashboard.

---

## Reliability Engine

The reliability engine wraps each workflow step with four additive controls. All behaviors are feature-gated and backward-compatible — existing workflows are unaffected unless you opt in.

### Budget Governor
Tracks token spend per step and per run. On budget breach:
- If a degradation path is available → truncates step output and continues (`degrade` transition)
- If no degradation path → aborts the step (`budget-exceeded` transition)

### Guardrails Hook
Runs a pre-step check before executing any step marked as risky. If the guardrails check blocks the operation, the step is transitioned to `guardrail-blocked` and the run fails that step cleanly.

### Confidence-Driven Escalation
After a step produces output, the reliability engine scores confidence. Low-confidence output routes into the existing approval flow — a human can review and approve or reject before the workflow continues.

### Recovery Transitions
Each failure mode is recorded as an explicit transition (not a silent crash). Steps retry up to a bounded limit before marking as `failed`. Transition records are queryable on the run detail.

---

## Adaptive Shadow

Adaptive Shadow records advisory routing recommendations alongside each run without changing execution. The router observes which model/route was used for each step and computes what it *would* recommend based on historical performance — logged as a shadow recommendation.

**This is read-only.** The actual route used does not change. Shadow recommendations accumulate over time and can be used to tune `ModelRouter` config.

Shadow data is surfaced in the **Persisted Runs** accordion under **Routing Recommendations**:

```
step_id          current_route       recommended_route    confidence
researcher       claude-sonnet-5   claude-opus-5      74%  (12 samples)
implementer      claude-sonnet-5   claude-sonnet-5    91%  (12 samples)
```

`fallback: true` means the router had insufficient data and used the default.

---

## Creating a workflow

```bash
agent007_workflow_create name="my-workflow" yaml="..."
```

Workflow YAML schema:
```yaml
name: my-workflow
description: What this workflow does

steps:
  - id: research
    agent: Researcher
    prompt: |
      Research the following: {{task}}
    output: research_findings

  - id: implement
    agent: Engineer
    depends_on: [research]
    prompt: |
      Based on: {{research_findings}}
      Implement: {{task}}
    output: implementation
```

Fields:
- `id` — unique step identifier
- `agent` — persona name to use for this step
- `prompt` — Tera template; `{{task}}` is always available; prior step outputs available by their `output` variable name
- `output` — variable name this step's result is stored under
- `depends_on` — list of step IDs that must complete first (omit for parallel execution)
