# PRD: Skill & Workflow Sharing

> Status: Draft — pending approval

## Executive Summary

Users build valuable skills and workflows in one project but can't easily share them with
other projects, teammates, or the community. This PRD defines a **layered sharing system**
that starts with a simple one-click "promote to global" action and grows toward Git-based
remote installs and cross-project dashboard copy.

## User Stories

### Tier 1 — Promote to Global
- **US-1**: As a developer, I want to promote a project-local skill to `~/.agent007/skills/`
  so it is available in all my future projects, via one click in the dashboard.
- **US-2**: As a developer, I want to demote a global skill back to project-local scope
  so it stops appearing in unrelated projects.

### Tier 2 — Bundle Export/Import
- **US-3**: As a developer, I want to export a set of skills and workflows as a
  `.a7bundle` file so I can share them with a teammate via email or Slack.
- **US-4**: As a developer, I want to import a `.a7bundle` file into a project so the
  skills and workflows are immediately available.
- **US-5**: As a team lead, I want to distribute a "team starter bundle" containing our
  standard code-review and commit-message skills to new team members.

### Tier 3 — Git URL Install
- **US-6**: As a developer, I want to run `agent007 skill install <url>` to download a
  skill from any HTTP URL or GitHub raw link.
- **US-7**: As a developer, I want a lock file (`installed.toml`) so I can reproduce my
  skill/workflow setup on a new machine with `agent007 sync`.
- **US-8**: As a developer, I want content-hash verification so I know an installed skill
  hasn't been tampered with.

### Tier 4 — Cross-Project Dashboard Copy
- **US-9**: As a developer, I want to open the dashboard, see a list of other running
  agent007 projects (via ports.toml), browse their skills, and copy one with a click.

## Functional Requirements

| ID   | Requirement                                                                 | Tier |
|------|-----------------------------------------------------------------------------|------|
| FR-1 | `POST /api/skills/:trigger/promote` — copy skill to global home             | 1    |
| FR-2 | `POST /api/workflows/:name/promote` — copy workflow to global home          | 1    |
| FR-3 | Dashboard "Promote" button on every skill and workflow card                 | 1    |
| FR-4 | `agent007 bundle export [--skill ...] [--workflow ...]` CLI command         | 2    |
| FR-5 | `agent007 bundle import <file>` CLI command                                 | 2    |
| FR-6 | Dashboard "Export Bundle" and "Import Bundle" buttons                       | 2    |
| FR-7 | `.a7bundle` format: JSON envelope with base64 asset contents                | 2    |
| FR-8 | `agent007 skill install <url>` — download and place in `~/.agent007/`       | 3    |
| FR-9 | `~/.agent007/installed.toml` lock file with source URL + sha256             | 3    |
| FR-10| `agent007 sync` — re-fetch all installed assets from their source URLs      | 3    |
| FR-11| Dashboard "Projects" panel listing peers from ports.toml                   | 4    |
| FR-12| Cross-project `GET /api/skills` proxy call to peer dashboard               | 4    |
| FR-13| Dashboard one-click "Copy from peer project" action                        | 4    |

## Non-Functional Requirements

- **NFR-1**: Bundle import must not overwrite existing files without user confirmation
- **NFR-2**: URL install must validate content hash before writing to disk
- **NFR-3**: Cross-project API calls must stay local (no internet for Tier 4)
- **NFR-4**: All operations must work offline except Tier 3 (URL install) and Tier 5

## Out of Scope (this PRD)

- Public hosted registry / marketplace (Tier 5) — separate PRD
- Conflict resolution UI for bundle import merges
- Access control / private skills
- Automatic sync on file change (watch mode)

## Success Metrics

1. Users promote skills/workflows within first session after feature launch
2. Bundle round-trip: export → import on fresh machine produces identical skills
3. `agent007 skill install <github_raw_url>` works end-to-end in < 3s
4. Cross-project copy works when both dashboards are in ports.toml
