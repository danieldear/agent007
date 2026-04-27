# Release Strategy

This document defines the current release policy for `agent007`.

## Current Approach

`agent007` uses a startup-style release model with one primary distribution channel:

```text
GitHub Releases
  + version tags
  + curated release notes
  + prebuilt binaries
  + checksums
  + curl-based installer
```

This is the intentionally simple path for now.

## Why This Model

We are still shipping major features quickly. The current priority is:

1. Keep release operations simple.
2. Make installation work on macOS and Linux with minimal friction.
3. Avoid maintaining multiple package ecosystems too early.
4. Preserve a clean user-facing release story while milestones are still moving fast.

## Distribution Policy

### Primary

GitHub Releases are the source of truth for installs.

Each release must include:

1. A git tag.
2. A GitHub Release entry.
3. Release notes written for humans.
4. Prebuilt binaries for supported targets named as:
   - `agent007-x86_64-unknown-linux-gnu.tar.gz`
   - `agent007-x86_64-apple-darwin.tar.gz`
   - `agent007-aarch64-apple-darwin.tar.gz`
5. `SHA256SUMS`.
6. A curl-installable shell script (`install.sh`).

### Deferred For Now

These are explicitly not part of the current release plan:

1. Homebrew tap.
2. `apt` / `.deb` packaging.
3. Package-manager-first distribution.
4. Complex release branching.

These can be added later if user demand justifies the maintenance cost.

## Install Philosophy

The primary install story should be:

```bash
curl -fsSL https://raw.githubusercontent.com/danieldear/agent007/main/scripts/install.sh | bash
```

The installer should:

1. Detect OS and architecture.
2. Download the correct release asset from GitHub Releases.
3. Verify checksums.
4. Install the binary into a reasonable user path.
5. Print next-step PATH guidance when needed.

Manual binary download from GitHub Releases remains the fallback path.

Specific version install:

```bash
curl -fsSL https://raw.githubusercontent.com/danieldear/agent007/main/scripts/install.sh | bash -s -- --version v0.1.0
```

## Automation

Release operations are automated with GitHub Actions:

- `.github/workflows/ci.yml`
  - Rust formatting and test checks.
  - Frontend dependency install + production build validation.
- `.github/workflows/release.yml`
  - Triggered by pushing a `v*` tag.
  - Builds release binaries for supported targets.
  - Produces per-asset checksums and consolidated `SHA256SUMS`.
  - Publishes release artifacts and generated release notes.

## Versioning

Public releases should use semantic versioning:

```text
0.MINOR.PATCH
```

Because the project is still pre-1.0:

1. `MINOR` means meaningful user-visible capability progress.
2. `PATCH` means fixes, hardening, documentation, and release polish.

## Milestones vs Releases

Milestones and public releases are related, but they are not the same thing.

```text
milestones
  -> internal delivery checkpoints

GitHub releases
  -> user-facing installable artifacts
```

Milestone language may appear in release titles or notes, but the long-term public contract should remain semver-centered.

## Release Classes

### Pre-release

Use when:

1. A milestone slice is ready for testing.
2. We want outside validation.
3. Known issues still exist.

Mark these as GitHub prereleases.

### Stable release

Use when:

1. The feature set is intentionally supported.
2. Docs are updated.
3. Install path is verified.
4. Core validation has passed.

## Minimum Release Checklist

Before publishing a release:

1. CI pipeline (`.github/workflows/ci.yml`) passes.
2. Release notes are written.
3. Known issues are updated.
4. Version tag is created.
5. GitHub Release artifacts are attached by `.github/workflows/release.yml`.
6. `SHA256SUMS` is present and validated against uploaded archives.
7. Curl installer is tested on at least one macOS and one Linux environment.

## Current Decision

The project stays on:

```text
Model A
-------
GitHub Releases + curl installer
```

We are deliberately deferring Homebrew, `apt`, and broader packaging work until the product surface is more stable.
