# ADR-0001: Same-Repository Registry with Immutable Release Artifacts

**Status:** Accepted
**Date:** 2026-06-18

## Context

The generic agent007 distribution should support software delivery, quality,
release, and council workflows without carrying every possible specialist domain.
Specializations need a discoverable, versioned, reversible distribution model.

agent007 already has two useful foundations:

- `.a7bundle` exports skills, workflows, personas, and tools with per-entry
  SHA-256 verification.
- Extension APIs already demonstrate preview/install/uninstall flows.

The existing `docs/registry.json` is a flat skill catalog, not a package registry.
The current `.a7bundle` implementation is JSON/text-only v1; the compressed v2
format is documented but not yet implemented.

## Decision

1. Keep official registry metadata and pack source in the agent007 repository.
2. Build pack artifacts in CI and publish immutable GitHub Release assets.
3. Pin manifest and artifact SHA-256 values in `registry/v1/index.json`.
4. Install packs into versioned global or project directories and activate them
   through `packs/lock.json` catalog overlays.
5. Keep previous versions for explicit rollback.
6. Ship an official-only registry first. Configuration may point to another
   registry, but third-party trust is explicit rather than automatic.
7. Reuse `.a7bundle` v1 initially and adopt v2 without changing the registry's
   package identity or lifecycle model.

The repository's example entry uses repository-relative URLs while the feature
is under review so local and pull-request verification can rebuild it. Its
SHA-256 pins still prevent silent substitution. A published pack version should
replace those bootstrap URLs with the immutable assets produced by the Publish
Domain Pack workflow.

## Why not download from `main` directly?

Mutable branch URLs make installs non-reproducible and allow content to change
without a version change. Release artifacts provide immutable URLs and rollback.
Keeping the source and index in the same repository still provides a single code
review and governance surface.

## Why catalog overlays instead of copying files?

Copying pack assets into `skills/`, `workflows/`, or `personas/` loses ownership
information and makes disable, update, conflict handling, and uninstall unsafe.
Versioned overlays preserve provenance and make activation a lockfile operation.

## Trust model

```text
maintainer-reviewed source
        |
        v
CI-built artifact ---- SHA-256 ---- registry version entry
        |                                  |
        +---------- immutable release -----+

client install
├── verify registry schema
├── verify manifest SHA-256
├── validate manifest and compatibility
├── verify artifact size and SHA-256
├── verify every .a7bundle entry hash
└── stage before lockfile activation
```

Signature verification can be added to the same version entry later. Integrity
pinning is mandatory from the first release.

## Consequences

### Positive

- The default catalog remains small and generic.
- Installs are deterministic, inspectable, and reversible.
- Project and global scopes use the same package model.
- The Hub and CLI share one lifecycle implementation.
- Deep domain systems can evolve independently from the agent007 binary.

### Negative

- Release automation and registry maintenance become required.
- `.a7bundle` v1 cannot carry binary assets.
- Catalog loaders must understand enabled pack overlays.
- Cross-pack dependency policy adds package-manager complexity.

## Follow-up decisions

- Add Sigstore identity and transparency-log verification after the integrity-
  pinned MVP is stable.
- Reconsider a separate registry repository only when contribution volume or
  permissions justify independent governance.
- Add private authenticated registries as a separate feature.
