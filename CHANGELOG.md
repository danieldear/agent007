# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for public releases.

## [Unreleased]

### Added

- Public release governance files (`SECURITY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SUPPORT.md`, `LICENSE`).
- GitHub Actions CI and release workflows for automated checks and tagged artifact publishing.
- `scripts/install.sh` curl installer with platform detection and checksum verification.

### Changed

- README and release strategy docs aligned with the GitHub Releases + curl installer path.
- Regression test fixture in `crates/testing/src/regression.rs` updated for deterministic baseline threshold behavior.

## [0.1.0] - 2026-04-27

### Added

- Initial public project baseline for `agent007` orchestration platform.
