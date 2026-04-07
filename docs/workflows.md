# Workflows

Workflows are multi-step agent pipelines defined as YAML files in `~/.agent007/workflows/`. Steps without `depends_on` run in parallel; steps with `depends_on` run after their predecessors complete.

## Running a workflow

**Via MCP (from your AI editor):**
```
agent007_workflow_run name="tdd" task="Add rate limiting to the API"
```

**Via CLI:**
```bash
agent007 workflow run tdd "Add rate limiting to the API"
```

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
