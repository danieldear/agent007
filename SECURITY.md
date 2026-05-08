# Security Policy

## Supported Versions

Security fixes are currently provided for the latest release line only.

| Version | Supported |
| --- | --- |
| latest `0.x` release | yes |
| older releases | no |

---

## Security Architecture Overview

This section is intended for security reviewers and enterprise IT teams evaluating agent007 for internal deployment.

### What agent007 is

agent007 is a **local MCP (Model Context Protocol) server** that runs as a process on the developer's own machine. It exposes a structured tool API to AI editors (Claude Code, Cursor, etc.) over stdio — the same transport used by every MCP server. It also serves a local web dashboard for task inspection and configuration.

It is **not** a cloud service, a SaaS product, or a relay. All state lives on the developer's filesystem under `.agent007/` in the project root or `~/.agent007/` globally.

---

### Data flows and external network calls

agent007 makes outbound HTTPS calls only in the following cases. All calls are user-initiated — nothing runs on a background timer or sends data without an explicit user action.

| Endpoint | When | What is sent | Purpose |
|---|---|---|---|
| `api.anthropic.com/v1/messages` | Every task/skill run that uses the Claude provider | The prompt and conversation context | Inference — the core function of the tool |
| `api.openai.com/v1/responses` | When the Codex/GPT provider is configured and a task runs | The prompt and conversation context | Inference (optional provider) |
| `api.github.com` | When the user triggers a Git PR operation | Repository name, branch, PR metadata | GitHub PR creation (user-initiated) |
| `raw.githubusercontent.com/danieldear/agent007/main/docs/registry.json` | When a user opens the Skill Registry tab in the dashboard | Nothing — GET request only | Fetches the public skill catalog so users can browse importable skills |
| `registry.npmjs.org`, `pypi.org`, `crates.io`, `formulae.brew.sh` | When a user runs a tool search in the Tools tab | Search query string | Discovers installable tools (user-initiated) |

**There is no telemetry, analytics, crash reporting, usage tracking, or any other background phone-home of any kind.** The word "telemetry" appears in the codebase only as a local file name (`retrieval-telemetry.json`) that records RAG retrieval quality metrics to the local session store — it is never read by, or sent to, any external service.

You can verify this by auditing all `reqwest::Client` usages in the codebase:

```bash
grep -rn "reqwest::Client\|\.get(\|\.post(" crates/ --include="*.rs" | grep -v "//\|#\[test"
```

Every call resolves to one of the endpoints listed above.

---

### Secrets and API key handling

- API keys (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`) are read from **environment variables only** at startup. They are never written to disk by agent007.
- Keys are held in-process memory and used only for the inference calls described above.
- The web dashboard API does not expose the configured API keys over any endpoint.
- The Zones system (see below) treats `.env` and `*.pem` files as **Forbidden** by default — agents cannot read them even if a prompt tries to direct them to do so.

---

### File system access controls — the Zones system

agent007 includes a compile-time enforced zone policy (`crates/zones/`) that classifies file paths into four access levels:

| Zone | Default patterns | Effect |
|---|---|---|
| `Forbidden` | `.env`, `*.pem`, `secrets/`, `keys/` | No read, write, or delete — hard block |
| `Sensitive` | Configurable | Read allowed, write/delete blocked |
| `Readonly` | Configurable | Read allowed, write/delete blocked |
| `Unrestricted` | Everything else | All operations allowed |

These rules are configurable per-project in `.agent007/config.toml` under `[zones]`. Every file operation goes through `ToolExecutor.check_zone()`, which calls the `ZoneChecker` and emits an audit log entry regardless of whether the operation is allowed or blocked.

---

### Audit logging

Every file operation checked through the Zones system is recorded to `.agent007/audit/audit.log` as newline-delimited JSON:

```json
{"ts":"2026-05-04T10:00:00Z","agent":"WorkerAgent","action":"write","path":"src/auth/login.rs","zone":"readonly","allowed":false,"blocked":true}
```

Fields: `ts` (ISO 8601 UTC), `agent` (agent identity), `action` (read/write/delete), `path`, `zone`, `allowed`, `blocked`.

---

### Web dashboard network binding

The web dashboard (`agent007 serve`) currently binds to `0.0.0.0:<port>` (default 8007), which means it is reachable from other machines on the same network segment. **There is no authentication layer on the dashboard in the current version.**

For enterprise deployments, the recommended mitigations until an auth layer is added (tracked in `docs/security-gaps.md`):

- Restrict access at the network/firewall level (allow only `127.0.0.1` → `localhost`)
- Run agent007 with `--no-dashboard` flag to disable the web server entirely (MCP-only mode)
- Place behind a reverse proxy with basic auth if remote access is needed

---

### MCP tool surface

The MCP server exposes approximately 44 tools to the connected AI editor. The tools that touch persistent state or execute actions include:

- `agent007_run` / `agent007_dispatch` — run AI tasks
- `agent007_git_commit` — stage and commit files in the local git repo
- `agent007_memory_write` — write to the local memory store
- `agent007_skill_create` — create new skill files
- `agent007_workflow_run` / `agent007_workflow_approve` — run multi-step pipelines with optional human approval gates

All of these are invoked **by the AI editor** in response to user prompts — they are not triggered autonomously. The AI editor (Claude Code, Cursor, etc.) must call them explicitly, and the editor's own permission system applies on top of agent007's zone checks.

---

### Dependencies

agent007 is written in Rust and uses only mainstream, widely-adopted crates. Key dependencies:

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime |
| `axum` | Web server |
| `reqwest` | HTTP client (rustls TLS, no OpenSSL) |
| `serde` / `serde_json` | Serialization |
| `rmcp` | MCP protocol implementation |
| `git2` | Git operations (libgit2 binding) |
| `globset` | Glob pattern matching for zone rules |
| `tracing` | Structured logging |

A full SBOM can be generated from the workspace lock file:

```bash
cargo install cargo-cyclonedx
cargo cyclonedx --format json
```

Dependency CVE scanning:

```bash
cargo install cargo-audit
cargo audit
```

---

### Known gaps and hardening roadmap

See [`docs/security-gaps.md`](docs/security-gaps.md) for a prioritized list of known security gaps and the planned work to address each one. This document is maintained by the project team and updated as gaps are closed.

---

## Reporting a Vulnerability

Do not open a public issue for undisclosed vulnerabilities.

Use one of the following private channels:

1. GitHub Security Advisory (`Security` tab -> `Report a vulnerability`) when enabled.
2. If advisory reporting is unavailable, open a private communication with maintainers through repository contact details and include `[security]` in the subject.

Please include:

- Affected versions and environment.
- Reproduction steps or proof of concept.
- Impact assessment.
- Suggested remediation, if known.

## Disclosure Process

1. Maintainers acknowledge reports within 3 business days.
2. We validate and classify severity.
3. A fix is prepared, tested, and released.
4. Public disclosure is published in release notes after a fix shifts.
