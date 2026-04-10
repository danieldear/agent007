# V2 M2 Reliability Implementation Guide

This document describes the production implementation of Milestone M2 in `agent007`.

## 1. What Was Added

M2 reliability is implemented in the workflow runtime with four additive controls:

1. Recovery transition engine with bounded retries and explicit transition records.
2. Budget governor with graceful degradation before abort.
3. Guardrails hook for risky operations.
4. Confidence-driven escalation into the existing approval flow.

All behaviors are feature-gated and backward-compatible by default.

## 2. Runtime Flow (ASCII)

```text
ready step
   |
   v
guardrails check
   | allow
   |-------------------------> block -> transition: guardrail-blocked -> fail step
   v
execute/model output
   |
   v
confidence check
   | low confidence
   |-------------------------> request approval -> existing approval path
   v
budget check
   | within limit
   |-------------------------> transition: continue
   |
   | exceeded + degradation available
   |-------------------------> truncate output -> transition: degrade -> continue
   |
   | exceeded + no degradation path
   |-------------------------> transition: abort -> fail run
```

## 3. Configuration Model

M2 now supports two additive configuration layers:

1. Global environment flags/tunables (process-level defaults).
2. Optional workflow-level overrides in workflow schema (`WorkflowDef.reliability`).

### 3.1 Workflow-Level Override Example

```toml
name = "feature"

[reliability]
enabled = true

[reliability.recovery]
enabled = true
max_step_retries = 3

[reliability.budget_governor]
enabled = true
max_degradations_per_run = 2
degrade_output_chars = 300

[reliability.guardrails]
enabled = true
terms = ["drop table", "rm -rf"]

[reliability.confidence]
enabled = true
low_terms = ["confidence: low", "uncertain"]
missing_requires_approval = true
```

### 3.2 Environment Flags

Use these environment variables to roll out M2 capabilities incrementally:

1. `AGENT007_RELIABILITY_ENABLED=1`
2. `AGENT007_RELIABILITY_RECOVERY=1`
3. `AGENT007_RELIABILITY_BUDGET_GOVERNOR=1`
4. `AGENT007_RELIABILITY_GUARDRAILS=1`
5. `AGENT007_RELIABILITY_CONFIDENCE_ESCALATION=1`

Tuning flags:

1. `AGENT007_RELIABILITY_MAX_STEP_RETRIES` (default: `2`)
2. `AGENT007_RELIABILITY_MAX_DEGRADATIONS` (default: `1`)
3. `AGENT007_RELIABILITY_DEGRADE_OUTPUT_CHARS` (default: `400`)
4. `AGENT007_RELIABILITY_GUARDRAIL_TERMS` (comma-separated)
5. `AGENT007_RELIABILITY_LOW_CONFIDENCE_TERMS` (comma-separated)
6. `AGENT007_RELIABILITY_CONFIDENCE_REQUIRE_ON_MISSING=1`

## 4. Structured Reliability Events

Workflow state now records both:

1. `reliability_transitions` (typed transition list).
2. `reliability_events` (event stream payloads, hosted parity).

Transition kinds:

1. `continue`
2. `retry`
3. `degrade`
4. `escalate-approval`
5. `abort`
6. `guardrail-blocked`

Each transition captures:

1. `step_id`
2. `kind`
3. `reason_code`
4. optional `detail`

Standalone workflow runs also emit `workflow-reliability-transition` run notes.
Hosted workflows now persist equivalent reliability event payloads in workflow state for analytics parity.

## 5. Guardrails Behavior

When guardrails are enabled, the runner/hosted engine checks rendered step context with normalized matching:

1. Rendered prompt content is preferred over raw templates.
2. Matching runs across normalized forms (spaced + compact) to catch obfuscated risky terms.
3. If a risky operation is detected, execution is blocked with deterministic reason codes.

Default risky terms include examples like:

1. `rm -rf`
2. `drop table`
3. `truncate table`
4. `delete from`
5. `format disk`
6. `wipe database`

## 6. Budget Governor Behavior

When budget would be exceeded and governor is enabled:

1. First preference: degrade output size (truncate to configured char limit) if degradation budget remains.
2. Fallback: abort with explicit reason.

This creates a deterministic degrade-before-abort path instead of immediate failure.

## 7. Confidence Escalation Behavior

When confidence escalation is enabled and low-confidence signals are present in step output:

1. Workflow requests approval even if `requires_approval` is not set on the step.
2. Existing approval decision flow (approve/edit/deny) remains the only gate.
3. Transition is logged as `escalate-approval`.

## 8. Backward Compatibility

With all reliability flags unset:

1. Existing workflow semantics are preserved.
2. Existing workflows do not require schema changes (new `reliability` block is optional).
3. Existing hosted and standalone APIs remain compatible.

## 9. Operational Rollout

Recommended rollout sequence:

1. Enable `AGENT007_RELIABILITY_BUDGET_GOVERNOR` in staging.
2. Enable `AGENT007_RELIABILITY_GUARDRAILS` with conservative term list.
3. Enable `AGENT007_RELIABILITY_CONFIDENCE_ESCALATION` for selected workflows.
4. Enable `AGENT007_RELIABILITY_RECOVERY` once retry behavior is validated.
5. Enable global `AGENT007_RELIABILITY_ENABLED` after verification.
