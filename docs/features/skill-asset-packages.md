# Feature: Skill Asset Packages (Folder Format Support)

**Status:** Implemented  
**Last updated:** 2026-05-03

---

## Summary

agent007 now supports both skill formats:

1. Flat markdown skills (`<name>.md`).
2. Directory-packaged skills (`<name>/SKILL.md` + optional assets/scripts).

No migration is required for existing flat skills.

---

## Supported Formats

### Format A: Flat file

```text
~/.agent007/skills/code-review.md
```

### Format B: Skill package directory

```text
~/.agent007/skills/code-review/
  SKILL.md
  assets/
  scripts/
  README.md
```

`SKILL.md` is the canonical manifest/template file for packaged skills.

---

## Runtime Behavior

Implemented loader behavior:

1. If entry is `*.md`, load as flat skill.
2. If entry is a directory, load `<dir>/SKILL.md` if present.
3. Directories without `SKILL.md` are ignored.

Implemented templating behavior:

1. `skill_dir` is injected into skill template context.
2. Packaged skills can reference local assets/scripts relative to `skill_dir`.

---

## Import/Copy Behavior

Implemented CLI and server behavior includes package support:

1. Skill add/import supports copying package directories.
2. Package validation requires `SKILL.md`.
3. Assets and scripts in package folders are preserved.

---

## What This Enables

1. Shipping richer reusable skills without flattening.
2. Bundling supporting files directly with prompts.
3. Cleaner reuse across projects and teams.

---

## Remaining Improvements

1. Optional package lint command for validating asset/script references.
2. Optional package signature/provenance metadata.
3. Better package metadata rendering in dashboard list views.
