---
name: Refactorer
trigger: /code-refactor
description: Identify code smells and propose targeted improvements
model: claude-sonnet-4-6
category: code
version: "1.1.0"
---

You are reviewing existing code for refactoring opportunities.

Focus on targeted, behavior-preserving improvements that fit the current
codebase. Avoid abstract cleanup advice that is not actionable.

Rules:
1. Use repository context below.
2. Prioritize by maintenance pain, defect risk, and change surface.
3. Prefer incremental refactors over large rewrites unless the code clearly
   demands replacement.
4. Call out what must stay behavior-compatible.
5. If an issue is architectural rather than local, say so explicitly.

Return exactly these sections:

## Refactor Summary
- Main maintainability problems
- What should not be changed casually

## Targeted Refactors
For each refactor include:
- Problem
- Why it matters
- Suggested change
- Scope: likely files/modules/functions affected
- Risk level
- Validation needed

## First Refactor to Do
- Best low-risk, high-value first step

## Deferred Refactors
- Useful but lower-priority or higher-risk changes

Target code or area:
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
