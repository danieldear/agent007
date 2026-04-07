# ADR-001: Rust as Implementation Language

**Date:** 2026-04-07  
**Status:** Accepted  
**Deciders:** agent007 core team

## Context

agent007 is a CLI tool that also doubles as an MCP server and LSP server. It needs to:

- Start fast (no perceptible cold-start in editor integrations)
- Ship as a single self-contained binary with no runtime dependencies for end users
- Handle concurrent async I/O (multiple MCP tool calls, streaming LLM responses, workflow state machines)
- Be memory-safe — it reads and writes user files, executes shell hooks, and manages session state

The initial design explored several mainstream language options before settling on a language.

## Decision

Implement agent007 in Rust using a Cargo workspace. The project is structured as 18 crates under `crates/` to enforce clean separation of concerns and enable incremental compilation.

## Rationale

- **Single binary distribution**: `cargo build --release` produces one statically-linked binary. Users `curl` it or `cargo install` it — no `pip install`, `npm install`, or runtime version managers required.
- **Memory safety without a GC**: The ownership model eliminates entire classes of bugs (use-after-free, data races) at compile time, which matters when the server is long-running inside an editor process.
- **Async performance**: `tokio` provides a battle-tested async runtime. Concurrent MCP tool dispatch, workflow step execution, and HTTP dashboard serving all run on the same lightweight runtime.
- **Strong type system**: Protocol types (MCP tool schemas, workflow YAML structures, skill frontmatter) are modeled as Rust enums and structs. `serde` + compile-time derives catch schema mismatches before shipping.
- **Ecosystem maturity**: `rmcp` for MCP, `tower-lsp` for LSP, `axum` for the web dashboard, `tera` for templates — all production-quality crates with active maintenance.

## Alternatives Considered

| Language | Reason Rejected |
|----------|----------------|
| **Python** | Slow startup (import time alone exceeds acceptable cold-start budget); distributing as a single binary requires PyInstaller/Nuitka (fragile); dependency resolution (`pip`) becomes the user's problem |
| **Go** | Seriously evaluated. Go produces small static binaries and has good async primitives. Rejected primarily because the Rust type system provides stronger correctness guarantees for the complex state machines in the workflow engine, and the team had existing Rust expertise |
| **Node.js / TypeScript** | Binary distribution via `pkg`/`bun` produces large bundles (30–100 MB); V8 startup overhead; no memory safety |

## Consequences

### Positive

- Users install a single `agent007` binary — zero runtime dependency surface
- Compile-time correctness: protocol schema changes break the build, not production
- Memory and CPU efficiency enables the MCP server to idle inside an editor with negligible overhead
- Cross-compilation (`cargo build --target`) enables publishing pre-built binaries for macOS (aarch64/x86_64), Linux, and Windows from CI

### Negative / Tradeoffs

- **Development complexity**: Lifetimes, ownership, and the borrow checker add friction for contributors unfamiliar with Rust
- **Compile times**: A full `cargo build --release` across 18 crates is slow (~60–120 s cold). Incremental builds and `sccache` mitigate this in practice
- **Steeper contribution barrier**: Potential contributors who know Python or JS must learn Rust before contributing meaningfully
- **Crate churn**: The Rust async ecosystem still evolves quickly; upgrading `tokio` or `rmcp` occasionally requires non-trivial migration work

## Related ADRs

- ADR-002 — MCP stdio transport (relies on Rust's `rmcp` crate and `tokio` async runtime)
- ADR-004 — Hosted-MCP workflow execution (WorkflowEngine implemented as a Rust async state machine)
