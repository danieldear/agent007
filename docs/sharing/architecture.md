# Architecture: Skill & Workflow Sharing

## Component Breakdown

```
┌─────────────────────────────────────────────────────────────┐
│                     Dashboard (Vue 3)                       │
│  SkillCard  ─► [Promote]  [Export]  [Copy from Project]     │
│  WorkflowCard ─► [Promote] [Export] [Copy from Project]     │
│  BundleImportDropzone                                       │
│  PeerProjectPanel (lists ports.toml entries)                │
└──────────────────┬──────────────────────────────────────────┘
                   │ HTTP / REST
┌──────────────────▼──────────────────────────────────────────┐
│                     Web API (Axum)                          │
│  POST /api/skills/:trigger/promote                          │
│  POST /api/workflows/:name/promote                          │
│  GET  /api/bundle/export?skills=...&workflows=...           │
│  POST /api/bundle/import                                    │
│  GET  /api/peers           ← reads ~/.agent007/ports.toml   │
│  GET  /api/peers/:port/skills  ← proxy to peer dashboard    │
└──────────────────┬──────────────────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────────────────┐
│                  crates/sharing (new crate)                 │
│  BundleBuilder   — serialize skills/workflows to JSON       │
│  BundleImporter  — validate + write to target home          │
│  PeerRegistry    — read ports.toml, probe liveness          │
└──────────────────┬──────────────────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────────────────┐
│              CLI Commands (crates/cli)                      │
│  agent007 bundle export [--skill X] [--workflow Y]          │
│  agent007 bundle import <file.a7bundle>                     │
│  agent007 skill install <url>                               │
│  agent007 sync                                              │
└─────────────────────────────────────────────────────────────┘
```

## Data Models

### `.a7bundle` Format (JSON)
```json
{
  "version": "1",
  "created_at": "2026-04-08T...",
  "source_project": "/Users/neo/workspace/agent007",
  "skills": [
    {
      "filename": "code-review.md",
      "content": "---\ntrigger: /code-review\n...",
      "sha256": "abc123..."
    }
  ],
  "workflows": [
    {
      "filename": "tdd.yaml",
      "content": "name: tdd\nsteps: ...",
      "sha256": "def456..."
    }
  ]
}
```

### `~/.agent007/installed.toml` (lock file)
```toml
[skills."code-review"]
source = "https://raw.githubusercontent.com/user/repo/main/code-review.md"
sha256 = "abc123..."
installed_at = "2026-04-08T..."

[workflows."tdd"]
source = "github:user/repo/workflows/tdd.yaml"
sha256 = "def456..."
installed_at = "2026-04-08T..."
```

## API Contracts

### Promote
```
POST /api/skills/:trigger/promote
→ 200 { "promoted_to": "/Users/neo/.agent007/skills/code-review.md" }
→ 409 { "error": "already exists globally" }
```

### Bundle Export
```
GET /api/bundle/export?skills=code-review,commit-msg&workflows=tdd
→ 200 Content-Type: application/json
     Body: { .a7bundle JSON }
```

### Bundle Import
```
POST /api/bundle/import
Content-Type: multipart/form-data (or application/json)
→ 200 { "imported_skills": [...], "imported_workflows": [...], "conflicts": [...] }
→ 409 { "conflicts": [{"name": "code-review", "action": "skip|overwrite"}] }
```

### Peer Discovery
```
GET /api/peers
→ [ { "project": "/path/to/project", "port": 8008, "alive": true } ]

GET /api/peers/8008/skills
→ proxied response from http://localhost:8008/api/skills
```

## Technology Choices

| Concern | Choice | Rationale |
|---------|--------|-----------|
| Bundle format | JSON (not tar.gz) | Human-readable, diffable, no binary deps |
| Hash algo | SHA-256 | Standard, in Rust stdlib via `sha2` crate |
| URL install | `reqwest` (already in workspace) | No new dependency |
| Peer proxy | Direct HTTP via `reqwest` | Simple, stays local |
| Lock file format | TOML | Consistent with rest of agent007 config |

## Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Bundle import overwrites custom skills | Conflict detection + user confirmation before write |
| URL install fetches malicious content | SHA-256 verification + prompt user to inspect before install |
| Peer proxy leaks sensitive data | Only proxy `/api/skills` and `/api/workflows` endpoints (read-only) |
| ports.toml stale entries | Probe liveness before showing peer; show "offline" for dead entries |

## Implementation Order (maps to PRD tiers)

1. **Tier 1** — `POST /api/skills/:trigger/promote` + dashboard button (1-2 days)
2. **Tier 2** — `crates/sharing` bundle builder/importer + CLI + dashboard (3-4 days)
3. **Tier 3** — `agent007 skill install <url>` + `installed.toml` lock (2-3 days)
4. **Tier 4** — peer discovery via ports.toml + dashboard panel (2-3 days)
