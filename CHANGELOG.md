# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for public releases.

## [Unreleased]

## [0.5.0] - 2026-05-25

### Added

- Structural repo graph v1 with persisted graph artifacts, file/symbol/module/doc nodes, and relationship edges for repo-aware analysis.
- Repo Intelligence lifecycle with empty-repo, baseline-only, enrichment-available, and enrichment-active readiness states.
- New ETR graph tools for build, refresh, symbol lookup, callers/callees, context bundles, doc links, impact radius, dependency paths, and usage graph queries.
- Dedicated Repo Intelligence dashboard page with graph health, readiness, install actions, capability notes, and an interactive graph workbench.
- Automatic structural-graph preflight for analysis and review flows in CLI and dashboard execution paths.
- Structural-intelligence milestone documentation and updated homepage copy aligned to the shipped feature set.

### Changed

- Dashboard now keeps Repo Graph and Repo Intelligence as compact summary surfaces while moving heavy graph exploration to a dedicated page.
- WebSocket/dashboard metric merging now preserves repo graph and readiness state instead of flickering when partial status updates arrive.
- Repo Intelligence install UX now separates actionable installs, recent install results, and non-actionable capability notes.
- Homepage product messaging and MCP config examples were corrected to match the current runtime and release behavior.

## [0.4.0] - 2026-05-22

### Added

- Public release governance files (`SECURITY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SUPPORT.md`, `LICENSE`).
- GitHub Actions CI and release workflows for automated checks and tagged artifact publishing.
- `scripts/install.sh` curl installer with platform detection and checksum verification.
- Hosted workflow self-submitting steps with injected `session_id` and `step_id` so step agents can submit outputs directly.
- Lazy output fetching via `agent007_workflow_get_output(session, key)` to reduce full-output injection into hosted workflows.
- Workflow heartbeat and staleness tracking with per-step liveness surfaced in workflow status and dashboard UI.
- Memory-backed step claims and optional inline token reporting for hosted workflow submissions.

### Changed

- README and release strategy docs aligned with the GitHub Releases plus curl installer path.
- Regression fixture behavior in `crates/testing/src/regression.rs` made deterministic.
- Release workflow now focuses Linux artifacts and provides explicit source-install guidance when macOS assets are unavailable.
- Workflow step state and prompt instructions now carry heartbeat metadata and stronger stale-step guidance.

## [0.3.1] - 2026-05-08

### Added

- Embedded Tool Runtime expansion with reusable built-ins for table selection, grouping, joins, metrics summaries, workflow outputs/health, log correlation, and deltas.
- LSP configuration management via dashboard API and config documentation.
- Security maturity roadmap document for phased hardening work.

### Changed

- Release workflow now supports manual dispatch testing and publishes cross-platform artifacts for Linux, macOS Apple Silicon, and Windows x64.
- Intel macOS installer path now fails fast with explicit source-install guidance instead of a missing-asset download failure.
- Documentation refreshed for ETR, LSP configuration, release behavior, and security planning.

## [0.1.0] - 2026-04-27

### Added

- Initial public project baseline for `agent007` orchestration platform.
