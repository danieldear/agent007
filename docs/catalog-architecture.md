# Catalog Architecture: Generic Core and Optional Domain Packs

## Decision

agent007's default catalog is domain-neutral. Project-specific analysis systems do
not belong in the catalog created by `agent007 init`, even when they are useful or
technically sophisticated. They belong in optional, namespaced domain packs.

The product boundary is:

```text
agent007
|
+-- Runtime foundation
|   +-- MCP and CLI
|   +-- context, memory, retrieval, and repo intelligence
|   +-- model routing, approvals, policies, and observability
|   +-- ETR deterministic tools
|
+-- Generic technical catalog
|   +-- Discover: brainstorm, ideation, council
|   +-- Define: PRD, architecture, milestones, project planning
|   +-- Deliver: implementation profiles from TDD to full-cycle delivery
|   +-- Verify: tests, debugging, code/security/performance/product review
|   +-- Ship: release readiness, PR follow-up, changelog, release
|
+-- Domain-neutral decision support
|   +-- evidence gathering
|   +-- alternatives and scenario analysis
|   +-- LLM council
|   +-- risk, uncertainty, and recommendation synthesis
|
`-- Optional domain packs (installed explicitly)
    +-- finance.*
    +-- travel.*
    +-- legal.*, health.*, science.*, or organization-specific packs
    `-- project-specific packs such as wifi-ftm.*
```

## What the generic technical catalog must support

### 1. Discovery and project formation

- Explore vague ideas without prematurely selecting a solution.
- Compare alternatives, constraints, users, risks, and evidence needs.
- Turn an approved direction into durable ideation notes, a PRD, architecture,
  milestones, and an executable project plan.

### 2. Architecture and delivery

- Understand an existing repository before proposing broad changes.
- Design architecture, interfaces, data flow, rollout, migration, and recovery.
- Implement features at different levels of ceremony.
- Keep milestones, requirements, implementation, and validation traceable.

### 3. Quality engineering

- Design tests from requirements and risk, not merely from existing code.
- Support TDD, regression testing, integration testing, end-to-end testing,
  accessibility testing, performance testing, and live smoke testing.
- Debug with explicit hypotheses, evidence, reproduction, fixes, and regression
  protection.

### 4. Review and assurance

- Review correctness, maintainability, architecture, UX, accessibility,
  performance, privacy, dependencies, and operational readiness.
- Include OWASP and relevant ecosystem standards where applicable.
- Rank findings by severity, confidence, evidence, user impact, and remediation.
- Separate a broad code review from a deep security audit while allowing the two
  to share common review primitives.

### 5. Shipping and follow-through

- Assess release readiness with explicit blocking and non-blocking findings.
- Produce changelogs, release notes, migration notes, and rollback plans.
- Babysit pull requests by monitoring CI, review threads, merge conflicts, and
  requested changes until the PR is merge-ready.
- Never report completion from a local patch alone when remote state matters.

### 6. General-purpose council

- Deliberate on ambiguous or high-impact questions across technical and
  non-technical domains.
- Preserve consensus, dissent, assumptions, risks, missing evidence, and next
  actions.
- Remain advisory; domain packs add specialized evidence and safety rules.

## Persona, skill, and workflow responsibilities

```text
Persona  = judgment policy
           mission, boundaries, evidence hierarchy, decisions, escalation

Skill    = reusable expert procedure
           triggers, inputs, tool choreography, artifacts, validation, stop rules

Workflow = orchestration
           dependencies, parallelism, retries, approvals, handoffs, completion
```

A workflow should compose personas and skills rather than duplicate their full
instructions. A persona should not become a workflow hidden inside a system
prompt. A skill should not own project-wide approval or release state.

## Resolving overlapping workflows

`tdd`, `sparc`, and `feature` all answer the high-level request "build this", but
they represent different delivery profiles rather than three unrelated products:

| Existing workflow | Delivery profile | Best fit |
|---|---|---|
| `tdd` | `delivery:tdd` | A bounded behavior or defect with tests first |
| `sparc` | `delivery:greenfield` | A new subsystem where specification and architecture must precede implementation |
| `feature` | `delivery:full` | Production delivery with research, reviews, approval gates, docs, and release sign-off |

Similarly, `brainstorm` and `ideation` are discovery profiles:

| Existing workflow | Discovery profile | Best fit |
|---|---|---|
| `brainstorm` | `discovery:quick` | Explore options and capture an approved direction |
| `ideation` | `discovery:full` | Produce PRD, architecture, and project plan |

The current names remain compatibility entry points. The target UX should offer
intent-based routing (`discovery`, `delivery`, `quality`, `release`, `council`)
and let the router or user select a profile. This reduces choice overload without
removing useful execution depth.

## Domain-pack contract

The contract below is implemented by the official v1 registry, CLI lifecycle,
catalog overlays, and Hub management surface. See [Domain Packs](domain-packs.md)
for commands, trust boundaries, authoring, and recovery.

Every optional domain pack should:

1. Use a namespace for skills, workflows, personas, tools, and memory.
2. Declare its version, compatibility range, dependencies, permissions, and
   data-source requirements in a manifest.
3. Install explicitly and remain absent from generic `agent007 init` output.
4. Keep specialized runtimes and datasets outside the core repository.
5. Define evidence freshness, uncertainty, safety, and approval policies.
6. Include evaluation fixtures and smoke tests.
7. Support clean enable, disable, upgrade, export, and uninstall operations.

### Finance pack

A finance pack may provide market research, financial statement analysis,
valuation, scenario modeling, portfolio-risk analysis, and trading-strategy
research. It must timestamp market data, cite sources, distinguish fact from
inference, disclose uncertainty, and require explicit approval before any external
or trade-execution action. Autonomous trading is not a default core capability.

### Travel pack

A travel pack may provide destination research, itinerary design, routing,
budgeting, reservations checklists, and disruption handling. Time-sensitive facts
such as entry rules, closures, schedules, availability, weather, and safety notices
must be refreshed from authoritative sources.

## Migration plan

```text
Phase 1  Catalog hygiene
         Remove project-specific RTT/FTM entries and genericize core examples.

Phase 2  Intent-based catalog
         Add discovery/delivery/quality/release profiles and compatibility aliases.

Phase 3  Persona and Skill Quality v2
         Add shared operating protocol, stronger schemas, linting, and evaluations.

Phase 4  Domain-pack lifecycle                                  IMPLEMENTED
         Manifest, registry, install/update/rollback/uninstall, overlays, and Hub.

Phase 5  First optional packs
         Build finance and travel as separate, policy-aware packages.
```

This separation keeps agent007 broadly useful while allowing deep specialization
without polluting every project's context, slash commands, or default catalog.
