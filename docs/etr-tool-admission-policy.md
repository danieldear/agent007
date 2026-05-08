# ETR Tool Admission Policy (Core + Optional Packs)

## Why this policy exists

`agent007` should optimize for:

- deterministic behavior
- low token usage
- low latency
- high reliability

Not every useful idea belongs in Core ETR.  
This policy keeps the runtime fast, stable, and maintainable.

`agent007` core is **domain-agnostic**.
Domain-specific logic belongs in optional packs/plugins, not core built-ins.

---

## North-star model

```text
                +----------------------+
                |   Candidate Tool     |
                +----------+-----------+
                           |
                           v
              +---------------------------+
              |  Scorecard Evaluation     |
              |  (value, risk, overlap)   |
              +-------------+-------------+
                            |
          +-----------------+-----------------+
          |                                   |
          v                                   v
 +-------------------------+       +--------------------------+
 | Promote to Core ETR     |       | Keep in Optional Pack    |
 | (always-on primitives)  |       | (domain-specific tools)  |
 +-------------------------+       +--------------------------+
```

---

## Core admission criteria (must pass)

A tool is eligible for **Core ETR** only if it clearly improves at least one:

1. **Token efficiency**  
   Replaces verbose shell/script parsing with compact structured output.
2. **Reliability**  
   Reduces failure/flake rate versus shell fallback.
3. **Latency**  
   Faster than equivalent shell + parser path.

And also:

4. **Low overlap** with existing ETR primitives.
5. **Stable schema** (input/output unlikely to churn).
6. **Safe behavior** (no hidden side effects; clear error contract).

---

## Optional-pack criteria

A tool should stay in an **Optional Pack** when it is:

- domain-specific (e.g., workflow internals, logs, security)
- valuable but not universally needed
- still evolving (schema/UX not stable yet)

---

## Tool scorecard (required before promotion)

Each candidate tool must include:

- **Primary use case**
- **Shell fallback currently used**
- **Expected token savings**
- **Expected reliability gain**
- **Expected latency gain**
- **Overlap analysis** (existing ETR tools)
- **Risk level** (Low/Medium/High)
- **Decision**: Reject / Optional / Core

---

## Lifecycle

```text
Proposal -> Experimental -> Optional Pack -> Core (if proven)
```

- **Experimental**: limited use, collect data.
- **Optional Pack**: useful and safe, but not universal.
- **Core**: repeatedly proven high-impact primitive.

---

## Core baseline (implemented)

Core now includes reusable domain-agnostic primitives across:

- JSON/query: `etr.json_extract`, `etr.json_query`, `etr.json_query_v2`
- tabular/select/aggregate: `etr.table_select`, `etr.table_stats`, `etr.group_count`
- time/join/metrics/delta: `etr.time_window_filter`, `etr.join_on_key`, `etr.metrics_summary`, `etr.delta_compare`
- workflow/log utilities: `etr.workflow_status_summary`, `etr.workflow_outputs_index`, `etr.workflow_step_health`, `etr.artifact_read`, `etr.logs_slice`, `etr.logs_correlate`
- file/text/search/math primitives: `etr.glob`, `etr.file_stat`, `etr.grep`, `etr.diff`, `etr.csv_slice`, `etr.text_extract`, `etr.math`, `etr.semantic_search_local`, `etr.policy_check`

## Next admission focus

Prioritize only where scorecard evidence shows clear gains:

1. higher-fidelity joins/windowing (without turning ETR into a full query engine)
2. structured workflow diagnostics that reduce repeated hosted-workflow polling cost
3. portability/security hardening for optional packs and third-party plugin tools

---

## Operating rule for agents

When an ETR tool can satisfy the task:

1. use ETR first
2. return compact structured output
3. avoid shell + temp-script parsing unless required

This keeps workflow context clean and reduces token waste.

---

## Plugin Admission Addendum (third-party tools)

Third-party/plugin tools are **not** auto-trusted.

They must pass policy checks before becoming available beyond local testing.

### Admission flow

```text
Plugin submission
  -> Quarantine (disabled by default)
  -> Validation (schema/safety/reliability)
  -> Limited pilot
  -> Approve as Optional Pack OR Reject
```

### Mandatory checks (all required)

1. **Schema quality**
   - explicit input/output schema
   - deterministic output shape
   - clear typed errors

2. **Safety**
   - no hidden side effects
   - explicit permission boundaries
   - path/network/process behavior declared

3. **Reliability**
   - reproducible results in repeated runs
   - bounded failure modes
   - graceful degradation/fallback notes

4. **Performance value**
   - measurable token or latency savings vs shell/script path
   - acceptable runtime overhead

5. **Overlap check**
   - must not duplicate Core ETR without clear advantage

6. **Observability**
   - emits enough metadata for debugging/audit
   - versioned tool identity

### Decision rules

- **Approve (Optional Pack)**: passes all checks + demonstrates value.
- **Reject**: fails any mandatory check.
- **Rework**: minor gaps with clear remediation path.

### Core protection rule

No third-party plugin tool can be promoted to Core ETR without:

- sustained usage evidence,
- low incident rate,
- and explicit maintainer approval.

Domain-specific tool families must remain plugin/optional-pack scoped unless generalized
into domain-agnostic primitives.
