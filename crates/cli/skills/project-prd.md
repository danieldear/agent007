---
name: PRD Writer
trigger: /project-prd
description: Product requirements document with user stories and constraints
model: claude-sonnet-4-6
category: project
version: "1.1.0"
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
