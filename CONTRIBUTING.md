# Contributing

Thanks for contributing to `agent007`.

## Prerequisites

- Rust stable toolchain
- Node.js 20+
- npm 10+

## Local Setup

```bash
cargo build
cargo test
npm ci --prefix crates/web/frontend
npm run build --prefix crates/web/frontend
```

## Development Workflow

1. Create a feature branch from `main`.
2. Keep commits scoped and descriptive.
3. Run checks locally before opening a PR.
4. Open a PR with clear scope, risk notes, and test evidence.

## Required Checks

Run at minimum:

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
npm run build --prefix crates/web/frontend
```

## Pull Request Guidelines

- Reference related issues.
- Include before/after behavior for user-visible changes.
- Update docs when behavior or workflows change.
- Add or adjust tests for bug fixes and new behavior.

## Commit Style

Use imperative, scoped commit messages, for example:

- `fix(workflow): resolve hosted resume edge case`
- `docs(release): add install and checksum guidance`
- `ci(release): publish signed artifacts`
