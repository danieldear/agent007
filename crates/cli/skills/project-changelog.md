---
name: Changelog Generator
trigger: /project-changelog
description: Generate changelogs grouped by type from git history
model: claude-sonnet-4-6
category: project
version: "1.2.0"
tags: ["release", "changelog", "documentation"]
---

You are generating a changelog for an existing project.

Produce a changelog that is useful to humans reading release notes. Do not
invent entries that are not supported by the provided input and repo context.

Rules:
1. Use the input and repo context below.
2. Group by meaningful change type.
3. Prefer user-facing descriptions over raw commit text.
4. Keep internal-only or unclear changes out of the headline sections unless
   they materially affect operators or contributors.
5. Call out uncertainty if the input is too incomplete to classify some entries.

Operational discipline:
- Use repo, memory, task, and LSP context before making broad claims.
- Prefer deterministic extraction first: ETR tools for grep/glob/file stats, JSON/table/log queries, metrics, diffs, and workflow status before ad-hoc shell parsing.
- Separate evidence from inference; cite files, commands, outputs, or prior step IDs when available.
- Produce decisions, risks, and next actions; avoid generic checklist filler.
- Do not claim validation ran unless it actually ran; otherwise name the exact validation to run.

Return exactly these sections:

## Summary
- What changed overall

## Features
- User-facing feature additions

## Fixes
- User-facing bug fixes or behavior corrections

## Docs and Developer Experience
- Docs, tooling, or workflow changes that matter

## Breaking or Notable Changes
- Anything that requires operator/developer attention

## Notes
- Classification caveats or missing context

Changelog input:
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
