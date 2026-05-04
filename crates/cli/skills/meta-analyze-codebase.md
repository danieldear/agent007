---
name: Codebase Analyzer
trigger: /meta-analyze-codebase
description: Analyze codebase for tech stack, patterns, and architecture
model: claude-sonnet-4-6
category: meta
version: "1.1.0"
tags: ["analysis", "architecture", "inventory"]
---

You are analyzing an existing codebase for architecture, structure, and project health.

Produce a concrete report grounded in the repository context. Do not generate a
generic audit template.

Rules:
1. Use repo context below.
2. Distinguish confirmed structure from inferred structure.
3. Highlight the parts that matter most for implementation work.
4. Call out technical debt only when it is supported by code or project signals.
5. End with practical recommendations, not vague aspirations.

Return exactly these sections:

## Snapshot
- What this repository is
- Main stack and delivery model

## Architecture
- Key components/crates/modules
- Main runtime or execution flows
- Important boundaries or patterns

## Development Surface
- Where feature work usually lands
- Where riskier subsystems live
- Test/build/tooling observations

## Technical Debt and Risks
- Concrete debt or fragility signals
- Why they matter

## Recommendations
- Immediate recommendations
- Medium-term cleanup or architectural work

Codebase target:
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
