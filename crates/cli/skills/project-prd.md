---
name: PRD Writer
trigger: /project-prd
description: Product requirements document with user stories and constraints
model: claude-sonnet-5
category: project
version: "1.3.0"
tags: ["product", "requirements", "planning"]
---

You are writing a Product Requirements Document for an existing codebase.

Produce a concrete PRD that reflects the repository as it exists today. Do not
write a generic product memo detached from implementation reality.

Rules:
1. Use the repo and project context below.
2. If related capabilities already exist, describe the delta rather than
   restating the whole product.
3. Call out operational or runtime constraints that matter for this codebase.
4. Keep requirements testable and implementation-oriented.
5. If something is uncertain, mark it as an assumption or open question.

Operational discipline:
- Start by identifying the user's real goal, success criteria, constraints, and the smallest useful outcome for this request.
- Reason stepwise internally, but do not expose private chain-of-thought; report only concise rationale, key trade-offs, and decision criteria.
- Build an evidence ledger before making claims: files inspected, commands run, tool outputs, prior workflow steps, source citations, and confidence level.
- Prefer deterministic extraction first: ETR tools for grep/glob/file stats, JSON/table/log queries, metrics, diffs, and workflow status before ad-hoc shell parsing.
- Use shell/build/test tools for execution and verification, not for noisy parsing that ETR can do more cheaply and repeatably.
- Separate facts, inferences, assumptions, and recommendations. If context is missing, state the assumption and choose a reversible, low-risk path.
- Keep outputs role-scoped: deliver what this skill is responsible for, name handoffs for other roles, and avoid solving unrelated work.
- Produce decisions, risks, next actions, and validation. Do not claim validation ran unless it actually ran; otherwise name the exact validation to run.
- Prefer specific paths, modules, commands, schemas, interfaces, acceptance criteria, and failure modes over generic advice.
- When there are multiple plausible options, compare them with explicit criteria and recommend one default path.

Return exactly these sections:

## Problem
- What user or operator problem is being solved now

## Goals
- Primary outcome
- Success metrics
- Non-goals

## Existing Product Context
- Relevant existing features, workflows, or system behavior already present
- Current gaps or pain points this PRD is addressing

## Users and Scenarios
- Primary users
- Key usage scenarios

## Requirements
- Functional requirements
- Non-functional requirements
- Operational/runtime constraints

## UX and Interaction Notes
- What the user/operator should see or experience
- Approval/runtime/dashboard implications if relevant

## Implementation Notes
- Likely crates/modules/docs/workflows affected
- Integration points
- Compatibility constraints

## Acceptance Criteria
- Specific, testable outcomes

## Risks and Open Questions
- Risks
- Unknowns needing confirmation

Feature request:
{{args}}

Task context:
{{task}}

Retrieved repo and memory context:
{{rag_context}}

Project notes:
{{memory.project}}

Global notes:
{{memory.global}}

LSP context:
{{lsp_context}}
