# Orchestration Depth Hardening Program (V3)

**Status:** Started (Phase 0 complete, Phase 1 in progress)  
**Started:** 2026-05-03  
**Owner:** core maintainers  
**Branch:** `codex/orchestration-depth-hardening`

---

## Why this program exists

agent007 already has strong local-first primitives (skills, workflows, personas, memory, tool registry),
but the current catalog quality is uneven:

- overlapping skills/personas
- workflow collisions across project/global homes
- inconsistent output contracts
- insufficient reliability semantics in many workflows

That makes orchestration *appear* powerful while reducing repeatability in real day-to-day engineering work.

---

## Program objective

Raise orchestration from "works often" to "reliable by design" through:

1. catalog quality controls
2. stricter workflow contracts
3. deterministic recovery and review gates
4. measurable acceptance criteria (quality, cost, latency, retries)

---

## Visual target state

```text
Before
------
Task -> Prompt-heavy workflow -> Variable output -> Manual cleanup

After
-----
Task
  -> typed workflow contracts
  -> parallel bounded workers
  -> reducer/synthesizer
  -> multi-gate validation (code/test/security/perf)
  -> optional rework loop
  -> release/eval gate
  -> deterministic output + scorecard
```

```text
Depth = Decision Quality x State Durability x Parallel Coordination x Recovery Quality
```

---

## Scope

### In scope

- Built-in workflows, built-in skills, built-in personas
- Catalog collision reduction and quality linting
- Reliability and evaluation policy defaults for core workflows
- Documentation of contract standards and lifecycle gates

### Out of scope (this iteration)

- Public plugin marketplace operations
- Full remote extension governance service
- Multi-tenant SaaS runtime

---

## Canonical workflow set (high-priority)

These become the “gold set” and receive strict contracts first.

1. `ideation` / `brainstorm` (idea framing, options, approval)
2. `feature` / `sparc` (architecture + execution)
3. `tdd` (test-first implementation)
4. `code-review` (parallel code/test/security/perf review)
5. `security-audit` / `log-analysis` (deep diagnosis)

Cross-cutting reusable component:

- `sub_research` pattern inside ideation/architecture/debug workflows

---

## Contract standards

### Skill contract (required quality bar)

- clear trigger and scope
- explicit output format section (`Return exactly...` or equivalent)
- tags for routing/discovery
- semver version
- non-trivial description (purpose + boundaries)

### Persona contract (required quality bar)

- clearly bounded role ownership
- allowed tools explicitly defined
- strong system prompt with decision and escalation rules
- preferred model non-empty

### Workflow contract (required quality bar)

- description present
- schema-valid steps with deterministic output fields
- reliability block
- eval gate block
- at least one approval gate for high-risk flows

---

## Phase plan

### Phase 0 — Baseline audit (started)

Deliverables:

- `agent007 catalog audit` CLI command
- collision visibility (skills/workflows)
- first-pass quality findings for skills/workflows/personas

Exit criteria:

- audit runs in CI/local
- produces human and JSON output
- fails on errors (optional fail-on-warn mode)

### Phase 1 — Catalog cleanup

Deliverables:

- remove/merge duplicate triggers and workflow names
- unify naming and tags
- archive low-value/overlapping assets

Exit criteria:

- collisions reduced to zero or justified exceptions
- all gold-set assets pass audit with no errors

### Phase 2 — Workflow depth upgrades

Deliverables:

- add reliability defaults (retry policy, guardrails)
- add eval gates for release-class workflows
- ensure reducer/gate flow in review-centric workflows

Exit criteria:

- no silent dead-ends in gold workflows
- measurable retry and recovery behavior present

### Phase 3 — Quality gates + release criteria

Deliverables:

- benchmark harness for representative tasks
- scorecard per run: quality/cost/latency/retry
- release block on major regressions

Exit criteria:

- release candidates include measurable quality report
- regression thresholds enforced

---

## KPIs

### Reliability KPIs

- workflow completion rate
- stuck run/session count
- mean time to recover (MTTR)
- retry success ratio

### Quality KPIs

- gate pass rate (`code`, `tests`, `security`, `performance`)
- rework loop frequency
- post-merge defect escape rate

### Efficiency KPIs

- median latency per workflow class
- tokens per successful run
- cost per successful run

---

## Operational policy

```text
No new skill/persona/workflow enters built-ins unless:
- use-case gap is explicit
- contract checks pass
- owner is assigned
- benchmark delta is non-negative
```

```text
Fix quality first, then expand catalog surface.
```

---

## Immediate next actions (current branch)

1. Ship `agent007 catalog audit` command (human + JSON output).
2. Run audit and capture baseline findings.
3. Prioritize first cleanup PRs by severity and blast radius.
4. Rewrite top workflows and personas to contract standard.

---

## Risks

- Over-tightening contracts can reduce creative flexibility for ideation tasks.
- Large prompt rewrites can regress existing workflow behavior.
- Cleanup across project/global homes can surprise users if not documented.

Mitigations:

- keep fail-on-warn optional initially
- perform staged rollout (advisory -> enforced)
- provide migration notes and compatibility aliases

---

## Decision log

- **2026-05-03:** Prioritize catalog quality and orchestration depth before adding net-new assets.
- **2026-05-03:** Start with audit visibility + contract standards rather than immediate mass rewrites.
