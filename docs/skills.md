# Skills

Skills are reusable prompt templates triggered by `/slash-commands`. They're Markdown files with YAML frontmatter stored in `~/.agent007/skills/` (global) or `.agent007/skills/` (project-local). Project-local skills override global ones with the same trigger.

Skills can be placed directly in the skills directory or organized into **subdirectories (skill folders)**. The loader scans recursively, so `~/.agent007/skills/my-team/review.md` is loaded the same as `~/.agent007/skills/review.md`. Folders are purely organizational; the trigger still controls routing.

## Frontmatter schema

```yaml
---
name: My Skill              # Display name (required)
trigger: /my-skill          # Slash command trigger (required)
description: Does X         # Short description shown in skill_list (required)
model: claude-sonnet-4-6    # Preferred model (optional, falls back to config default)
version: "1.0.0"            # Semantic version (optional, defaults to "1.0.0")
---
Your prompt template here. Use {{args}} for user-provided arguments.
```

The template body is rendered with [Tera](https://keats.github.io/tera/). The `{{args}}` variable contains whatever the user passed after the trigger.

## Running a skill

**Command-style dispatch (Codex-friendly):**
```
agent007_dispatch command="$agent007 skill /my-skill my arguments here"
agent007_dispatch command="$agent007 /my-skill my arguments here"
```

**Via MCP tool (from your AI editor):**
```
agent007_skill_run trigger="/my-skill" args="my arguments here"
```

**Via CLI:**
```bash
agent007 skill run /my-skill "my arguments here"
```

`agent007_dispatch` is additive convenience. Direct skill tools still work.

## Installing skills

```bash
# From GitHub (raw content)
agent007 skill install github:owner/repo/path/to/skill.md

# From any HTTPS URL
agent007 skill install https://example.com/my-skill.md
```

Validation: the installer fetches the content, parses frontmatter, and checks for required fields (`name`, `trigger`) before writing to `~/.agent007/skills/`.

## Creating skills

Interactive wizard:
```bash
agent007 skill create
# or via MCP:
agent007_skill_wizard action="save" name="..." trigger="..." description="..." template="..."
```

When a skill/workflow is created or imported (CLI, MCP, or web dashboard), agent007
automatically refreshes Claude slash-command files in `.claude/commands` (project scope)
or `~/.claude/commands` (global scope). You no longer need to rerun `agent007 init`
just to register a new skill/workflow trigger.

## Listing skills

```bash
agent007 skill list
# Output:
# [v1.0.0] /code-document — Generate API docs, architecture docs, and inline documentation
# [v1.0.0] /dev-architect — Design system architecture from requirements
# ...
```

## Built-in skills (core set + optional specializations)

All built-in skills ship compiled into the binary and are available immediately after `agent007 init`.

| Trigger | Name | Description |
|---------|------|-------------|
| `/code-document` | Documentation Writer | Generate API docs, architecture docs, and inline documentation |
| `/code-optimize` | Performance Optimizer | Profile analysis and performance optimization suggestions |
| `/code-refactor` | Refactorer | Identify code smells and propose targeted improvements |
| `/code-security-audit` | Security Auditor | Security audit covering OWASP, dependencies, and threat modeling |
| `/code-test-gen` | Test Generator | Generate comprehensive test suites with edge cases |
| `/dev-architect` | Architect | Design system architecture from requirements |
| `/dev-debug` | Debugger | Systematic debugging with hypothesis-driven investigation |
| `/dev-pr-review` | PR Reviewer | Thorough pull request review with actionable feedback |
| `/dev-tdd` | TDD Coach | Test-driven development cycle (red-green-refactor) |
| `/meta-analyze-codebase` | Codebase Analyzer | Analyze codebase for tech stack, patterns, and architecture |
| `/meta-create-agent` | Agent Creator | Guided wizard to create a custom agent persona |
| `/project-changelog` | Changelog Generator | Generate changelogs grouped by type from git history |
| `/project-plan` | Project Planner | Break features into tasks with estimates and dependencies |
| `/project-prd` | PRD Writer | Product requirements document with user stories and constraints |
| `/project-release` | Release Manager | Version strategy, release notes, and rollback planning |

## Hooks on skill execution

When a skill runs via `agent007_skill_run` (including dispatch-routed skill calls), the `on_skill_execute` hook fires with:
- `HOOK_SKILL` — the trigger name
- `HOOK_ARGS` — the args passed to the skill

See [configuration.md](configuration.md) for hook setup.
