# Feature: Tool Registry for Hosted and Local Execution

**Status:** Implemented (V1)  
**Area:** `crates/cli/`, `crates/skills/`, `crates/web/`, hosted execution flow  
**Decision date:** 2026-04-14  
**Updated:** 2026-05-02

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

## Current V1 Implementation

Implemented in web/API and sharing layers:

```text
1. global + project-local discovery
   - ~/.agent007/tools
   - .agent007/tools
   - project precedence over global

2. package + legacy discovery
   - manifest packages (TOOL.yaml / tool.toml, etc.)
   - legacy flat scripts in tools root

3. web API for management
   - GET /api/tools
   - GET /api/tools/:name
   - POST /api/tools
   - DELETE /api/tools/:name
   - POST /api/tools/:name/test

4. deterministic test invocation
   - runtime dispatch (shell/python/node/binary)
   - timeout handling
   - stdout/stderr/exit_code capture
   - argument validation against manifest schema

5. sharing/export closure improvements
   - skill/workflow tool references pull associated tool files
   - supports path refs and named refs (tool:<name>)
```

Dashboard integration now includes a dedicated Tools view for list/create/edit/delete/test flows.

## V2 Additions (Implemented)

```text
1. Remote registry discovery
   - crates.io search
   - npm registry search
   - GitHub repository search

2. Provider import wrappers
   - crates import (cargo install wrapper)
   - npm import (npm install -g wrapper)
   - github import (git clone wrapper)
   - local import (copy local binary/script into package)

3. Quarantine + approval gate
   - imported tools default to quarantined
   - test execution is blocked until approval
   - approval endpoint refreshes trust state

4. Hash pinning
   - imported tools can pin SHA-256
   - execution verifies hash when pinning enabled
   - mismatch forces re-approval workflow

5. Optional skill generation
   - import can auto-create companion `/use-<tool>` skill
   - generated trigger is persisted in tool state metadata
```

---

## Next Phasing

### Phase 2 — Hosted Orchestration Integration

```text
- explicit hosted planner support for choosing registry tools
- tighter policy around when to auto-execute vs ask for approval
- richer run/audit linking from workflow steps to tool invocations
```

### Phase 3 — Sharing UX + Packaging

```text
- stronger bundle UX for tool packages and dependency visibility
- import/export conflict handling by package version/hash
- first-party example tool packs
```

### Phase 4 — Governance + Hardening

```text
- approval policies
- permission classes
- stronger audit trails
- timeout/retry policies
```

---

## Known Gaps

```text
- no remote registry/distribution model (local filesystem only)
- no package version solver yet
- trust boundary still depends on local operator discipline
- hosted workflow auto-tool selection is still heuristic, not policy-learned
```

---

## Decision

V1 is active and usable. Continue hardening through policy, auditability, and better hosted orchestration behavior while keeping the structured package model.
