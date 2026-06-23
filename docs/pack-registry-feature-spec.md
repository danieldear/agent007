# Feature Specification: Official Domain-Pack Registry

**Status:** Implemented; pending release
**Target:** agent007 0.7.x
**Owner:** agent007 maintainers

## Problem

agent007 needs deep specialist capabilities without adding every specialist
persona, skill, workflow, tool, and runtime to every project. The default catalog
must stay generic while users can discover and activate additional capabilities
on demand.

## Product decision

The official registry metadata and pack source live in the agent007 GitHub
repository. CI builds immutable `.a7bundle` artifacts and publishes them as
GitHub Release assets. Registry entries pin both manifests and artifacts by
SHA-256. The CLI and Hub use the same pack lifecycle APIs.

```text
GitHub repository
├── registry/v1/index.json
├── registry/v1/*.schema.json
├── packs/<pack-id>/
│   ├── pack.toml
│   ├── skills/
│   ├── workflows/
│   ├── personas/
│   └── tools/
└── GitHub Actions
    └── validate -> build -> hash -> release

agent007 home
└── packs/
    ├── cache/registry.json
    ├── lock.json
    └── <pack-id>/<version>/
        ├── pack.toml
        ├── artifact.a7bundle
        ├── install.json
        ├── skills/
        ├── workflows/
        ├── personas/
        └── tools/
```

## Functional requirements

### Discovery

- Search packs by ID, name, description, category, and tags.
- Display available versions, compatibility, contents, permissions, and trust
  metadata before installation.
- Cache the registry and support explicit offline reads.

### Installation

- Support global and project scopes.
- Resolve compatible semantic versions and transitive pack dependencies.
- Reject dependency conflicts and cycles.
- Verify the manifest SHA-256 before parsing it.
- Verify artifact size and SHA-256 before extracting it.
- Reuse the existing `.a7bundle` per-entry hash verification and path traversal
  protection.
- Stage extraction before activating a version.
- Preserve older installed versions for rollback.

### Lifecycle

- List, enable, disable, update, rollback, and uninstall packs.
- Prevent disabling or uninstalling a dependency required by another installed
  pack.
- Maintain a machine-readable lockfile for deterministic activation.
- Treat enabled pack assets as catalog overlays without copying them into the
  user's editable global or project asset directories.
- Refresh generated IDE slash commands after activation changes.

### Hub

- Show registry packs and installed state.
- Search and inspect pack contents and permissions.
- Install, enable, disable, update, rollback, and uninstall global packs.
- Require the Hub mutation token for lifecycle changes.
- Clearly identify that Hub lifecycle actions use global scope. Project-scoped
  lifecycle is available through the CLI in the first release; a project scope
  selector is reserved for a later Hub iteration.

### Publishing

- Validate registry and manifest schemas in CI.
- Build a pack artifact from its source directory.
- Require registry size and hashes to match the built artifact.
- Publish immutable release assets only after validation succeeds.

## Non-functional requirements

- **Security:** reject path traversal, invalid identifiers, malformed schemas,
  hash mismatches, incompatible versions, and unapproved external actions.
- **Reliability:** use staging directories and atomic lockfile writes; a failed
  download must not activate a partial install.
- **Performance:** use a 15-minute registry cache by default and avoid reading
  disabled pack trees during normal catalog loading.
- **Compatibility:** preserve the existing generic catalog and `.a7bundle` v1
  importer. Pack format evolution remains independent from registry schema
  evolution.
- **Recoverability:** retain previous version directories until uninstall and
  expose explicit rollback.

## Out of scope for the first release

- Open community publishing without maintainer review.
- Autonomous execution of financial trades or other consequential external
  actions.
- A hosted registry service or package analytics backend.
- Automatic trust of arbitrary third-party registries.
- Binary `.a7bundle` v2 publishing; the current text-safe v1 format remains the
  initial transport until v2 is implemented.

## Acceptance criteria

1. A fresh `agent007 init` contains no optional pack assets.
2. `agent007 pack search` can read the official registry and an offline cache.
3. Installing the example pack creates only a versioned pack directory and a
   lockfile entry.
4. The example skill appears in CLI/MCP catalog listing only while the pack is
   enabled.
5. Tampered manifests, artifacts, and bundle entries are rejected.
6. Project packs override global packs for catalog lookup without overwriting
   manually maintained project assets.
7. Update preserves the previous version and rollback reactivates it.
8. Dependency conflicts, cycles, and unsafe removal are rejected with clear
   errors.
9. Hub lifecycle APIs require mutation authorization and return structured state.
10. Registry validation, Rust tests, fresh-init smoke tests, and Hub API smoke
    tests pass in CI.

## Definition of done

- Architecture decision and schemas are documented.
- CLI, core overlays, Hub API/UI, and publishing workflow are implemented.
- Unit and integration tests cover the complete lifecycle and failure paths.
- README and user documentation explain authoring, publishing, installation,
  scopes, trust, offline behavior, and recovery.
