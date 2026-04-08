# Ideation: Skill & Workflow Sharing

> Status: Brainstorm — not yet approved for implementation

## Problem Statement

agent007 skills (`.md` prompt templates) and workflows (`.yaml` pipelines) are powerful
but siloed. Each project has its own `.agent007/` folder. There is a global `~/.agent007/`
that is shared across all projects on one machine, but there is no mechanism to:

- Share a skill you built with a teammate
- Re-use a workflow you built in Project A inside Project B on a different machine
- Distribute a curated set of skills to a team
- Discover community-built skills

## Goals

1. **Copy/promote** a skill or workflow from a project to the global `~/.agent007/` in one click
2. **Bundle export/import** — pack selected assets into a portable file
3. **Git URL install** — `agent007 skill install <url>` (like `curl | sh` but tracked)
4. **Cross-project copy** — dashboard UI to copy from another running project (via ports.toml)
5. **Future: public registry** — marketplace for community discovery

## Current Architecture (relevant)

```
~/.agent007/                  ← global home (shared across all projects on this machine)
  skills/                     ← global skills
  workflows/                  ← global workflows

/project/.agent007/           ← project-local home (found by walking up from CWD)
  skills/
  workflows/

Path resolution (agent007_home()):
  AGENT007_HOME env > project-local .agent007/ > ~/.agent007/
```

The `ports.toml` registry (`~/.agent007/ports.toml`) already maps CWD → port for all
running dashboards. This is a **discovery mechanism we can leverage**.

## Explored Options

### Option A: Promote to Global (already partially possible via copy)
Move a project-local skill/workflow to `~/.agent007/` → available in all projects.
- **Pro**: Zero new infrastructure, works today with a file copy
- **Con**: No sharing with others, machine-local only
- **Effort**: Low (just a dashboard button + API endpoint)

### Option B: Bundle Export/Import
Export selected skills+workflows as a `.a7bundle` file (tar.gz or JSON).
Import on another machine by unpacking.
- **Pro**: Offline, no internet required, shareable via Slack/email
- **Con**: Manual, no versioning, no discoverability
- **Effort**: Medium (CLI + dashboard UI)

### Option C: Git URL Install
```sh
agent007 skill install https://raw.githubusercontent.com/user/repo/main/my-skill.md
agent007 workflow install github:user/repo/workflows/tdd.yaml
```
Files downloaded and placed in `~/.agent007/` (global) or `.agent007/` (project-local).
A `.a7lock.toml` tracks installed assets with their source URL + content hash.
- **Pro**: Versioned, reproducible, works with GitHub gists, any HTTP server
- **Con**: Requires internet; no curation/discovery
- **Effort**: Medium-High (CLI command, lock file, hash verification)

### Option D: Cross-Project Copy via Dashboard
`ports.toml` knows all running agent007 projects → dashboard "Import from Project" picker.
Select a peer project → browse its skills → click "Copy here".
- **Pro**: Excellent UX, no files to transfer manually
- **Con**: Only works when both dashboards are running; no persistence
- **Effort**: High (API cross-calls, peer discovery, UI)

### Option E: Public Registry / Marketplace
A hosted `registry.agent007.dev` where users publish and discover skills.
```sh
agent007 skill publish my-skill --tag code-review
agent007 skill search "git commit message"
agent007 skill install @community/git-commit-msg
```
- **Pro**: Maximum discoverability, community-driven
- **Con**: Requires hosting, trust/moderation, account system
- **Effort**: Very High (separate service)

## Recommended Direction (Layered)

Build in tiers — each tier is independently useful and feeds into the next:

```
Tier 1 (Fast win):   Promote to Global
                     Dashboard: "Make Global" button on any skill/workflow

Tier 2 (Offline):    Bundle Export/Import
                     CLI + Dashboard: export/import .a7bundle files

Tier 3 (Remote):     Git URL Install + Lock File
                     CLI: agent007 skill install <url>
                     .agent007/installed.toml tracks installed assets

Tier 4 (Discovery):  Cross-project Dashboard Copy
                     Uses ports.toml for peer discovery

Tier 5 (Community):  Public Registry
                     Long-term, requires separate hosting decision
```

## Open Questions for Human Input

1. Should the global `~/.agent007/` act as the "promoted" tier, or should we introduce
   a separate "shared" tier (e.g., a team-local shared dir)?

2. For bundles: JSON (human-readable, diffable) vs tar.gz (binary, handles large assets)?

3. Should installed assets (from URLs) be frozen in `.agent007/installed.toml` or
   always re-fetched on `agent007 sync`?

4. Privacy: some skills may contain proprietary prompts. Should export require
   an explicit "mark as shareable" flag?

5. For the public registry (Tier 5): self-hosted option vs. agent007-hosted?
