# Milestones: Skill & Workflow Sharing

## Milestone 1 — Promote to Global ✦ Fast Win
**Goal**: One-click promote from project-local → global `~/.agent007/`
**Effort**: ~1 day

| Feature | Description | Complexity |
|---------|-------------|------------|
| `POST /api/skills/:trigger/promote` | Copy skill file to global home | Low |
| `POST /api/workflows/:name/promote` | Copy workflow file to global home | Low |
| Dashboard "Promote" button | Add to SkillCard + WorkflowCard UI | Low |
| Conflict handling (already exists globally) | Return 409 + toast warning | Low |

**Exit Criteria**: User can click "Make Global" on a skill in the dashboard and it
immediately appears in all projects.

---

## Milestone 2 — Bundle Export / Import ✦ Offline Sharing
**Goal**: Pack skills+workflows into a portable `.a7bundle` file; import on any machine
**Effort**: ~3 days

| Feature | Description | Complexity |
|---------|-------------|------------|
| `crates/sharing` new crate | `BundleBuilder`, `BundleImporter`, SHA-256 | Medium |
| `GET /api/bundle/export` | Serialize to JSON bundle | Medium |
| `POST /api/bundle/import` | Validate + write with conflict detection | Medium |
| `agent007 bundle export` CLI | Select skills/workflows to include | Low |
| `agent007 bundle import` CLI | Import from file path | Low |
| Dashboard export/import UI | Export button + file drop zone | Medium |

**Exit Criteria**: Export a bundle from Project A, import on a fresh machine into
Project B — all skills/workflows appear identically.

---

## Milestone 3 — Git URL Install + Lock File ✦ Remote Sharing
**Goal**: Install skills from any URL; track with a lock file for reproducibility
**Effort**: ~2 days

| Feature | Description | Complexity |
|---------|-------------|------------|
| `agent007 skill install <url>` | Download + hash verify + write | Medium |
| `agent007 workflow install <url>` | Same for workflows | Low |
| `~/.agent007/installed.toml` | Lock file with source + sha256 | Medium |
| `agent007 sync` | Re-fetch all installed assets | Low |
| GitHub shorthand `github:user/repo/path` | URL resolver | Low |

**Exit Criteria**: `agent007 skill install https://raw.githubusercontent.com/...` 
downloads, verifies, and registers the skill. `agent007 sync` re-installs all on
a fresh machine.

---

## Milestone 4 — Cross-Project Dashboard Copy ✦ UX Delight
**Goal**: Browse and copy skills from other running agent007 dashboards on the same machine
**Effort**: ~2 days

| Feature | Description | Complexity |
|---------|-------------|------------|
| `GET /api/peers` | Read ports.toml, probe liveness | Low |
| `GET /api/peers/:port/skills` | Proxy to peer dashboard | Low |
| Dashboard "Projects" panel | Show peer projects with live/offline status | Medium |
| One-click copy from peer | Call promote on peer's skill to local | Medium |

**Exit Criteria**: Two projects running simultaneously — user can open Project A's
dashboard and copy a skill from Project B with a single click.

---

## Dependency Graph

```
M1 (Promote)
    │
    ▼
M2 (Bundle)  ────► M3 (URL Install)
                        │
                        ▼
                   M4 (Cross-project)
```

M1 and M2 are independent of M3/M4.
M4 builds on the `ports.toml` peer discovery introduced in the port registry feature.

## Definition of Done

- [ ] All PRD functional requirements implemented and tested
- [ ] Unit tests for `crates/sharing` (bundle round-trip, conflict detection, hash verify)
- [ ] Integration test: bundle export → import produces identical files
- [ ] Dashboard UI tested with both light/dark themes
- [ ] `installed.toml` lock file documented in `docs/configuration.md`
- [ ] CLI help text updated for new commands

## Suggested Execution Order

Run M1 alone first (fastest value). Then M2 and M3 in parallel (independent). M4 last.
