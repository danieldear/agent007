# ADR-005: Skills as Markdown Files with YAML Frontmatter

**Date:** 2026-04-07  
**Status:** Accepted  
**Deciders:** agent007 core team

## Context

Skills are reusable prompt templates that can be invoked via a `/slash-command` trigger. They need to:

- Be **human-readable and human-writable** — developers should be able to create and share skills without understanding the agent007 internals
- Carry **structured metadata** (trigger, model override, version, description) alongside the prompt body
- Support **dynamic content** — the prompt body needs to interpolate user arguments and context variables
- Be **shareable and installable** — a skill should be a single file that can be committed to a GitHub repo and installed with a URL reference

The format must balance machine parseability with the experience of writing and reading a skill file in a text editor.

## Decision

Skills are **Markdown files** (`.md` extension) with a **YAML frontmatter block** (delimited by `---`) and a **Tera template body**. Files live in `~/.agent007/skills/` (global) or `.agent007/skills/` (project-local).

Canonical skill file structure:

```markdown
---
name: Code Reviewer
trigger: /code-review
description: Full code review with actionable feedback
model: claude-sonnet-4-6
version: 1.0.0
---
You are a senior engineer performing a thorough code review.

Review the following: {{args}}

Focus on:
- Correctness and logic errors
- Security vulnerabilities
- Performance implications
- Maintainability

Provide specific, actionable feedback with line references where possible.
```

The `trigger` field defines the `/slash-command` used to invoke the skill. The body is a Tera template where `{{args}}` is replaced with the user's input at invocation time.

## Rationale

- **Frontmatter familiarity**: YAML frontmatter is the de facto standard for metadata in developer-facing content files — Jekyll, Hugo, Gatsby, Docusaurus, and Obsidian all use it. Developers encounter this pattern constantly and understand it without documentation.
- **Single-file portability**: Metadata and prompt live in one file. A skill can be `curl`-installed, emailed, or committed to a repo as a single artifact. No separate config file to keep in sync.
- **Tera templates are expressive**: `{{args}}` covers the common case, but Tera supports conditionals, loops, filters, and `{% if %}` blocks — useful for skills that adapt their behavior based on the presence or content of arguments. The same template engine is used elsewhere in agent007, reducing dependencies.
- **Markdown rendering**: The body renders naturally in GitHub, GitLab, and any Markdown-aware viewer. Skill files are readable documentation as well as executable prompts.
- **Installable from GitHub**: `agent007 skill install github:owner/repo/path/to/skill.md` fetches and installs a skill directly. The single-file format makes this trivial to implement and audit.

## Alternatives Considered

| Alternative | Reason Not Chosen |
|-------------|------------------|
| **TOML config + separate prompt file** | Two files per skill complicates distribution (must keep files co-located), sharing (must share both), and version control (two files to diff). The frontmatter pattern solves this cleanly |
| **JSON** | Not human-readable for prompt bodies. Writing a 500-word prompt inside a JSON string with `\n` escapes is error-prone and unpleasant to edit |
| **Pure Markdown with HTML comments for metadata** | `<!-- trigger: /code-review -->` — not a recognized standard, no parser support, fragile |
| **Embedded Rust structs (compile-time skills)** | Would make skills non-user-extensible. Users cannot add or modify skills without recompiling agent007. Rejected as incompatible with the extensibility goal |
| **Separate YAML files** | `.yaml` files with a `prompt:` key containing the template. More verbose than frontmatter + body, and loses the Markdown rendering benefit |

## Consequences

### Positive

- Skills can be written, shared, and installed as single `.md` files — minimal friction for the skill ecosystem
- `version` field (defaults to `"1.0.0"` if omitted) enables forward-compatible skill evolution
- Project-local skills in `.agent007/skills/` override global skills of the same trigger — enabling per-project customization
- The Markdown body is rendered readably in GitHub when a skill is published in a repository

### Negative / Tradeoffs

- **`serde_yaml` + Tera dependencies**: Both are pulled in for skill parsing and rendering. These are already present for workflow definitions (ADR-003) so there is no net new dependency.
- **Frontmatter parsing edge cases**: A skill file that accidentally contains `---` inside a code block in the body can confuse the frontmatter parser. The parser uses the first `---`/`---` pair only, mitigating most cases.
- **`version` field ignored at runtime**: The `version` field is stored and displayed but not yet used for compatibility checking or upgrade warnings. This is intentional (YAGNI) but may need revisiting as the skill ecosystem grows.
- **Tera syntax in prompt body**: Users who write `{{` in their prompt for non-template reasons (e.g., showing a Handlebars example) must escape it as `{{"{{"}}`. This is a rare but real footgun.

## Related ADRs

- ADR-003 — YAML for workflow definitions (YAML is also used here, in frontmatter form)
- ADR-006 — Synchronous hook execution (hooks fire on the same lifecycle events as skill invocations)
