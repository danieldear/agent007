# Feature: Skill Asset Packages (Folder Format Support)

**Status:** Planned  
**Area:** `crates/skills/`, `crates/cli/` (import command)  
**Decision date:** 2026-04-14

---

## Background

agent007 currently represents every skill as a single flat Markdown file:

```
~/.agent007/skills/<trigger>.md
```

The file contains a YAML frontmatter block (metadata, model, tags, etc.) followed by a Tera template body.

The Claude Code skill ecosystem uses a richer folder convention:

```
~/.agent007/skills/<skill-name>/
    SKILL.md          ← frontmatter + template (equivalent to today's flat file)
    assets/           ← optional: few-shot examples, config snippets, prompt partials
    scripts/          ← optional: helper shell scripts, jq filters, etc.
    README.md         ← optional: human-readable documentation
```

Because agent007 only understands flat files, importing a community skill that ships as a folder requires manual flattening — a friction point that breaks the "grab from GitHub, run immediately" promise.

---

## Goal

Support **both formats** transparently so the system works regardless of how a skill was authored or where it came from:

- Existing flat `.md` skills continue to work without any changes.
- Community skills in the folder format can be imported and used directly.
- Power users who author their own skills can use either format depending on complexity.

---

## Two Formats — Side by Side

### Format 1 — Flat file (current, remains supported)

```
~/.agent007/skills/code-review.md
```

Best for: simple prompt templates with no external assets. No migration needed.

### Format 2 — Skill directory package (new)

```
~/.agent007/skills/code-review/
    SKILL.md          ← required; same frontmatter + Tera template as today
    assets/           ← optional; static files the template can reference
    scripts/          ← optional; helper executables
    README.md         ← optional
```

The canonical file inside a skill directory is always `SKILL.md`. The directory name is the skill package name; the `trigger` field in `SKILL.md` frontmatter is what the skill is invoked by (same as today).

---

## Detection Logic

`SkillDispatcher` scans `~/.agent007/skills/` and applies this rule on each entry:

| Entry type | Rule |
|---|---|
| `<name>.md` (file) | Load as flat skill — existing behavior |
| `<name>/` (directory) | Look for `<name>/SKILL.md`; load that if present; ignore the directory if `SKILL.md` is absent |

Both formats coexist in the same directory. No migration of existing skills is required.

---

## Import UX

The `agent007 skill import <url>` command (and the dashboard's Import Skill flow) needs to handle both cases:

| What the URL points to | Expected behavior |
|---|---|
| A raw `.md` file | Download and save as a flat skill — current behavior |
| A GitHub folder / archive URL | Download the full directory tree and place it under `~/.agent007/skills/<name>/` |
| A GitHub repo root (entire skill repo) | Same as folder — extract into `skills/<name>/` |

The user should not need to know or care which format the skill author used.

---

## Asset Referencing

When a skill template (in `SKILL.md`) needs to reference a bundled asset, it should be able to do so via a relative path resolved against the skill's own directory. For example:

```
{{skill_dir}}/assets/few-shot-examples.txt
```

The exact Tera variable name (`skill_dir`, `asset_path`, etc.) is a design decision for implementation time. This only applies to folder-format skills.

---

## Shared Tools Folder (optional, deferred)

A separate but related idea: a `~/.agent007/tools/` directory for scripts that are **shared across multiple skills** rather than bundled per-skill. Skills would declare dependencies via frontmatter:

```yaml
requires_tools:
  - extract-json
  - format-diff
```

This is intentionally deferred. It introduces tool versioning and cross-skill dependency questions that are out of scope for this feature. The per-skill `scripts/` folder in the folder format covers the immediate need. If shared tools become a clear pattern in practice, they can be added later without changing the format.

---

## Non-goals

- Changing anything about how flat `.md` skills are loaded, stored, or invoked.
- Any versioning or lock-file mechanism for skill assets.
- Implementing the shared `~/.agent007/tools/` pool (deferred).
- Changing the Tera templating engine or frontmatter schema.

---

## Open Questions

1. **Canonical filename:** Should the required file inside a skill directory be `SKILL.md` (matching Claude Code convention) or `<trigger>.md` (matching the flat file convention)? `SKILL.md` is preferred for ecosystem compatibility but needs a final decision.
2. **Import URL detection:** How does the importer distinguish a GitHub folder URL from a raw file URL? GitHub's raw URL format differs from tree/blob URLs — detection heuristics need to be defined.
3. **Asset referencing variable name:** What Tera variable exposes the skill directory path to the template? Needs to be defined before implementation.
4. **Listing/display:** When `agent007 skill list` or the dashboard lists skills, should folder-format skills show differently (e.g., a package icon or asset count)?

---

## Migration

None required. Flat `.md` skills are never invalidated. The only change is that the loader gains the ability to read an additional format. A user with 20 existing flat skills sees zero change.
