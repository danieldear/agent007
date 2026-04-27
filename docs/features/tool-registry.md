# Feature: Tool Registry for Hosted and Local Execution

**Status:** Deferred / Next Plan  
**Area:** `crates/cli/`, `crates/skills/`, `crates/web/`, hosted execution flow  
**Decision date:** 2026-04-14

---

## Why This Exists

agent007 already has:

```text
skills
-> reusable reasoning/prompt templates

workflows
-> reusable orchestration

memory
-> reusable context
```

What it does **not** yet have is a first-class way to reuse **deterministic execution capability**.

Today, when a hosted LLM is asked to do something like:

```text
- flash a BSP
- run a log analyzer
- perform project setup
- run a repo-specific git hygiene sequence
- execute a device diagnostic script
```

it often has to:

```text
1. reconstruct the steps
2. reconstruct the command shape
3. guess arguments and preconditions
4. spend tokens explaining and re-deriving
5. risk drift across runs and operators
```

That is the gap this feature is meant to close.

---

## Goal

Introduce a first-class **tool registry** so agent007 can discover and invoke reusable project/global tools instead of repeatedly re-deriving the same operational steps.

The intended outcome is:

```text
reason once
-> package as tool
-> reuse across tasks, agents, and teammates
```

---

## Scope Model

The registry should follow the same precedence model as skills/personas:

```text
~/.agent007/tools/   -> global tools
.agent007/tools/     -> project-local tools
```

Resolution rule:

```text
project-local tool wins over global tool with the same name
```

This supports both:

```text
global reusable tools
---------------------
- git-branch-clean
- log-tail-errors
- repo-health-check

project-specific tools
----------------------
- adb-flash-bsp
- setup-board-env
- parse-bootlog
- sync-firmware-assets
```

---

## This Is Not Just "A Scripts Folder"

A plain scripts directory is not enough.

The hosted LLM needs each tool to be:

```text
discoverable
described
invocable
bounded
auditable
```

So each tool should be treated as a small package, not just a loose shell file.

Example shape:

```text
tool-name/
  TOOL.yaml
  run.sh
  README.md
  assets/          # optional
```

At minimum, the manifest should define:

```text
- name
- description
- entrypoint
- arguments schema
- environment requirements
- working directory policy
- timeout
- output contract
- safety / mutability classification
```

Without that metadata, the LLM cannot reliably decide:

```text
- when to use the tool
- how to call it
- whether it is safe to run
- how to interpret the result
```

---

## Product Value

### 1. Lower token spend

Instead of re-deriving procedural work every time:

```text
task
-> hosted LLM reasons
-> tool executes deterministically
-> hosted LLM reasons on result
```

That is much cheaper than repeatedly generating long command sequences.

### 2. Less operational drift

The command for a recurring operation lives in one place.

```text
before
------
same task asked 10 times
-> 10 slightly different command sequences

after
-----
same task asked 10 times
-> same named tool, same execution contract
```

### 3. Better team reuse

This is especially valuable for:

```text
- onboarding
- project setup
- hardware bring-up
- mobile/device workflows
- log analysis
- environment diagnostics
```

One person codifies the tool once; the team reuses it.

### 4. Better hosted execution

This fits hosted-MCP particularly well:

```text
hosted skill/workflow
-> decide whether tool is appropriate
-> invoke tool
-> inspect stdout/stderr/exit code/artifacts
-> continue reasoning
```

That is a stronger model than forcing the host LLM to be both:

```text
planner + command historian + shell cookbook
```

---

## Relationship to Existing Concepts

```text
skills
------
how to think

tools
-----
how to do a deterministic operation

workflows
---------
how to sequence thinking and doing
```

This distinction matters.

A good rule of thumb:

```text
"summarize this codebase"
-> skill

"flash this device"
-> tool

"investigate failure, flash device if needed, gather logs, summarize findings"
-> workflow using skills + tools
```

---

## Safety Model

If tools are exposed to the hosted LLM, they need explicit safety classes.

Suggested classes:

```text
readonly
--------
inspection, parsing, diagnostics

local-write
-----------
derived artifacts, caches, generated reports

project-mutating
----------------
git branch ops, codegen writes, workspace-changing actions

privileged / risky
------------------
device flashing, deploys, destructive cleanup, remote actions
```

This matters for:

```text
- approval policy
- auditability
- operator trust
- future permission controls
```

---

## Suggested V1 Scope

The smallest useful version is:

```text
1. global + project-local discovery
2. manifest format
3. list + inspect + invoke flow
4. stdout/stderr/exit-code capture
5. basic safety classification
6. audit/event recording
```

That is enough to make the feature real without over-designing it.

---

## Deferred Phasing

### Phase 1 — Registry Foundation

```text
- .agent007/tools and ~/.agent007/tools discovery
- manifest parsing
- precedence rules
- tool list / inspect APIs
```

### Phase 2 — Invocation

```text
- execute named tool with validated inputs
- capture stdout/stderr/exit code
- surface results to hosted workflows/skills
```

### Phase 3 — Sharing

```text
- import/export of tool packages
- team reuse and onboarding
- package docs and examples
```

### Phase 4 — Governance

```text
- approval policies
- permission classes
- stronger audit trails
- timeout/retry policies
```

---

## Why It Is Deferred

This is a strong feature, but it is not the current highest priority.

Current priority remains:

```text
fix product/runtime bugs first
then expand the reusable execution model
```

So this should stay documented as a planned feature until the current bug backlog is reduced.

---

## Non-goals for Now

```text
- implementing the registry immediately
- package version solving
- remote package distribution design
- generic plugin system ambitions
- replacing skills or workflows with tools
```

---

## Decision

Keep this as a documented next-plan item.

When work begins, build it as a **structured tool registry**, not as an unstructured shared scripts folder.
