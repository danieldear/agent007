# Security Gaps — Hardening Roadmap

This document lists known security gaps in the current codebase, their severity for enterprise/office deployment, and what work is needed to close each one. It is maintained by the project team. Gaps are closed by removing them from this list and updating `SECURITY.md` accordingly.

**Last updated:** 2026-05-21
**Current version:** 0.3.1

---

## Why this document exists

This project was flagged by an enterprise security review citing weak code, missing authentication, and potential IP exfiltration risk. Every gap in this document maps directly to one or more of those concerns. Gaps are rated on two axes: **severity** (blast radius if exploited) and **effort** (engineering cost to close).

---

## Summary Table

| # | Gap | Severity | Effort | Status | Category |
|---|---|---|---|---|---|
| 1 | Web dashboard has no authentication | Critical | Medium | Partial | Auth |
| 2 | Web server binds `0.0.0.0` by default | Critical | Low | Fixed | Network |
| 3 | Path traversal in memory key parameter | High | Low | Open | Input Validation |
| 4 | SSRF in skill discovery source expansion | High | Medium | Open | SSRF |
| 5 | Skill import can write outside intended directory | High | Low | Open | Path Traversal |
| 6 | Skill execution sandbox unenforced post-approval | High | High | Open | Execution |
| 7 | Provider credentials can appear in logs | High | Low | Open | Secrets |
| 8 | Data sent to third-party LLM providers (IP risk) | High | High | Open | IP / Data |
| 9 | No request body size limits (DoS) | Medium | Low | Fixed | DoS |
| 10 | Skill registry fetches from unpinned `main` branch | Medium | Low | Open | Supply Chain |
| 11 | Extension installer has no signature verification | Medium | High | Open | Supply Chain |
| 12 | Zone checker is opt-in, not enforced by default | Medium | Medium | Open | Access Control |
| 13 | No corporate identity / SSO integration | Medium | High | Open | Auth |
| 14 | No secrets vault integration | Low | High | Open | Secrets |
| 15 | No SBOM published with releases | Low | Low | Open | Supply Chain |
| 16 | `agent007_git_commit` has no mandatory approval gate | Low | Low | Open | Access Control |
| 17 | No dependency vulnerability scanning in CI | Medium | Low | Open | Supply Chain |

---

## Gap Details

---

---

> **Reading order for a security reviewer:** Gaps 1–8 are the ones that caused the enterprise rejection. Read those first. Gaps 9–17 are real but lower urgency.

---

### Gap 1 — Web dashboard has no authentication
**Severity: High**

**What the problem is:**
The web dashboard (`agent007 serve`) serves the dashboard UI and all REST API endpoints with no authentication. Any request that can reach the port is accepted and acted on — there is no bearer token, session cookie, or basic auth check.

**What an attacker could do:**
Anyone on the same network segment (office LAN, shared Wi-Fi, corporate network) who knows or discovers the port can: trigger task runs, read memory/skills/personas, export bundles, write new skills, and manage the MCP server registry — all without credentials.

**Where it is in the code:**
`crates/web/src/server.rs` — `into_router()`. No auth middleware is applied to any route.

**Workaround until fixed:**
- Use `--no-dashboard` flag to disable the web server entirely (recommended for MCP-only use)
- Restrict port access at the firewall/network level
- Run behind a localhost-only reverse proxy with basic auth if remote access is needed

**Work needed to close this gap:**
- Add a configurable static bearer token (set via `AGENT007_DASHBOARD_AUTH_TOKEN` env var or `config.toml`)
- Apply the token check as an Axum middleware layer before all dashboard/static/API/websocket routes
- Optionally: add an `--auth-token <token>` CLI flag to `serve` command
- Document the token requirement in `SECURITY.md` and `docs/configuration.md`

**Progress as of 2026-05-21:**
- Added optional dashboard auth with `AGENT007_DASHBOARD_AUTH_TOKEN`.
- Auth accepts `Authorization: Bearer <token>`, browser-friendly Basic auth, and `x-agent007-token`.
- `/health` and `/api/health` remain unauthenticated for local health checks.
- Still **partial**, not closed, because auth is opt-in and needs config-file/CLI UX plus docs before enterprise hardening can treat it as a default control.

---

### Gap 2 — Web server binds `0.0.0.0` by default
**Severity: High** (closely related to Gap 1)

**What the problem is:**
`TcpListener::bind("0.0.0.0:{port}")` binds all network interfaces. On a developer laptop on a corporate network, this means the dashboard is reachable from any other machine on the same subnet.

**Where it is in the code:**
`crates/web/src/server.rs`, lines 332, 377 — `format!("0.0.0.0:{port}")`.

**Workaround until fixed:**
Use `--no-dashboard` or firewall the port.

**Work needed to close this gap:**
- Change the default bind address to `127.0.0.1` (localhost only)
- Add a `--bind <addr>` CLI flag to `serve` and `serve-web` commands so operators who need LAN/remote access can opt in explicitly
- Update `docs/configuration.md` with the flag documentation

**Progress as of 2026-05-21:**
- Default dashboard bind host is now `127.0.0.1`.
- Operators can opt into another host with `AGENT007_DASHBOARD_HOST`.
- Remaining follow-up: document the env var and consider a typed CLI/config field for managed deployments.

---

---

### Gap 3 — Path traversal in memory key parameter
**Severity: High**

**What the problem is:**
In `memory_delete_handler` (`crates/web/src/api.rs`), the `scope` parameter is validated for `..`, `/`, and `\`. The `key` parameter is only checked for null bytes. If `resolve_existing_key_path` does not canonicalize the full resolved path and re-anchor it to the store root, a key value of `../../../some/other/file` can reach arbitrary filesystem paths.

**What an attacker could do:**
Delete or overwrite files outside the memory store directory, including config files, credentials, or other agent data.

**Where it is in the code:**
`crates/web/src/api.rs` — `memory_delete_handler` and `memory_get_handler`. `crates/memory/src/store.rs` — `resolve_existing_key_path`.

**Work needed to close this gap:**
- In `resolve_existing_key_path`: after constructing the full path, call `std::fs::canonicalize` (or equivalent) and assert the result starts with `self.root`. Return an error if the path escapes the root.
- Extend the key validation in `memory_delete_handler` to also reject keys containing `/`, `\`, and `..`.
- Add a test: key `"../escape"` must return an error, not a filesystem path.

---

### Gap 4 — SSRF in skill discovery source expansion
**Severity: High**

**What the problem is:**
`expand_skill_discovery_sources` (`crates/web/src/api.rs`) fetches arbitrary URLs after a prefix check for `https://github.com/` or `https://raw.githubusercontent.com/`. It then parses that Markdown and follows links found in it — a second-order fetch. The prefix check is evaluated on the string before DNS resolution and before following HTTP redirects.

**Attack vectors:**
- An open redirect on `github.com` itself (GitHub has had these historically) redirects to an internal network address.
- A crafted Markdown catalog page at a legitimate GitHub URL contains links to internal-network addresses that pass the prefix check.
- The response size is unbounded — a large Markdown response causes memory exhaustion.

**Where it is in the code:**
`crates/web/src/api.rs` — `expand_skill_discovery_sources`, `extract_github_urls_from_markdown`, `fetch_text_async`.

**Work needed to close this gap:**
- Set a hard response size limit on `fetch_text_async` (e.g., 512 KB max).
- Set a connect and read timeout on the `reqwest::Client` used for discovery fetches.
- After following redirects, validate the final URL (not the original) still matches the allowlist.
- Limit the number of catalog-expanded URLs followed per source (currently capped at 40, which is reasonable — verify it holds after redirects).
- Add a config option to disable catalog expansion entirely for deployments that need strict source control.

---

### Gap 5 — Skill import can write outside intended directory
**Severity: High**

**What the problem is:**
`write_imported_skill` and `generate_tool_skill` construct a filesystem write path from a `sanitize_file_stem` call on the skill trigger/name. If `sanitize_file_stem` is insufficiently strict, a crafted skill manifest can name a file that resolves outside the skills directory.

Package skills (subdirectory installs) are especially risky — the package directory name comes from the skill URL path and is used directly in `skills_dir.join(package_name)`.

**What an attacker could do:**
Write a file to an arbitrary path on the filesystem, overwriting existing files. Combined with a crafted YAML/TOML payload, this could overwrite `config.toml`, an existing skill, or a shell config file.

**Where it is in the code:**
`crates/web/src/api.rs` — `write_imported_skill`, `skill_import_handler`, `generate_tool_skill`.

**Work needed to close this gap:**
- After constructing the final write path, call `Path::canonicalize` on the parent directory and assert it is strictly within `skills_dir`. Return an error before any write if it is not.
- Reject package directory names that contain `/`, `\`, `..`, or start with `.`.
- Add a test: a skill trigger of `"../../etc/malicious"` must be rejected, not written.

---

### Gap 6 — Skill execution sandbox unenforced post-approval
**Severity: High**

**What the problem is:**
The approval workflow pauses before a skill runs for the first time. Once approved, a skill's `system_prompt` can reference MCP tools, issue arbitrary shell commands (if shell tools are enabled), or access the full filesystem. There is no per-skill tool allowlist enforced at execution time — the approval is a one-time gate, not a runtime constraint.

A malicious or compromised skill (e.g., imported from a catalog that later adds a backdoor) continues to execute with full privileges after the original approval.

**What an attacker could do:**
- Exfiltrate memory contents, config files, or API keys via a skill that makes outbound HTTP calls.
- Modify other skills or workflows on disk.
- Use the `agent007_git_commit` MCP tool to silently commit malicious code to the repo.

**Work needed to close this gap:**
- Add an `allowed_tools` list to each skill's frontmatter (already present in persona TOML — mirror for skills).
- At execution time, restrict the MCP tool set available to a skill to its declared `allowed_tools`.
- Treat a missing or empty `allowed_tools` as a prompt for the user to define one during the approval step.
- Document that re-approval is required if a skill's source content changes.

---

### Gap 7 — Provider credentials can appear in logs
**Severity: High**

**What the problem is:**
Provider configuration structs (containing API keys) pass through the readiness check and provider status layers. If a `tracing` call captures a provider config struct via `{:?}` or `{:#?}` debug formatting, the API key appears in the log output. The `fix(cli): redact ollama readiness source` commit in the history indicates this has already occurred at least once.

**Where to check:**
- `crates/web/src/api.rs` — all `tracing::*` call sites that reference provider config, readiness response, or request structs
- Any `derive(Debug)` on structs that contain `api_key`, `token`, or `secret` fields

**Work needed to close this gap:**
- Audit every `derive(Debug)` struct that could hold a credential. Override the `Debug` impl to redact sensitive fields, or use a wrapper type like `Redacted<String>` that prints `[REDACTED]`.
- Add a CI lint or unit test that constructs a provider config with a known dummy key, formats it with `{:?}`, and asserts the key string does not appear in the output.
- Review all `tracing::debug!` and `tracing::trace!` calls in provider and config modules.

---

### Gap 8 — Data sent to third-party LLM providers (IP / data exfiltration risk)
**Severity: High (enterprise blocker)**

**What the problem is:**
This is the primary reason enterprise reviewers cite IP leakage risk. Every skill run, workflow step, and agent persona call sends data to a third-party LLM provider (Anthropic, OpenAI, etc.). This data includes:
- The skill/workflow system prompt (which may contain internal business logic)
- The full conversation context, which accumulates memory entries, run history, and user inputs
- Any documents or code snippets passed as context
- Memory store contents read for retrieval-augmented steps

For an enterprise, this means proprietary workflows, internal data, and business-sensitive context are leaving the organization's network boundary on every inference call.

**What an attacker or competing party could do:**
This is less about external attack and more about contractual and regulatory exposure:
- Violation of data residency requirements (GDPR, SOC 2, HIPAA depending on domain)
- Unintended disclosure of trade secrets embedded in skill prompts
- Provider terms of service may allow training on API data depending on tier

**Work needed to close this gap:**
Short-term (required for any enterprise pilot):
- Add an `[enterprise]` config block with `allowed_providers = ["ollama"]` — when set, block inference calls to any non-listed provider and surface a clear error.
- Add a data classification field to skills (`data_classification = "internal" | "public"`) and refuse to run `internal`-classified skills against external providers.
- Document in `SECURITY.md` which data leaves the machine and to where.

Longer-term:
- First-class Ollama and local model support so an org can run fully air-gapped.
- Prompt content filtering before send: strip patterns matching internal naming conventions, secrets patterns, or user-defined regexes.
- Audit log every outbound inference call: timestamp, provider, model, token count, skill/workflow that triggered it (no prompt content in the audit log itself).

---

### Gap 9 — No request body size limits
**Severity: Medium**

**What the problem is:**
The Axum router has no `DefaultBodyLimit` layer. Endpoints that accept JSON bodies (`RuntimeMessageRequest`, skill save, workflow save, persona save, skill import) will read an unbounded amount of data from the connection before deserializing. An unauthenticated caller (Gap 1) can send a multi-gigabyte body and exhaust memory or disk.

**Where it is in the code:**
`crates/web/src/server.rs` — `into_router()`. No body limit layer is applied.

**Work needed to close this gap:**
```rust
// Add to router construction in server.rs
use axum::extract::DefaultBodyLimit;
let router = router.layer(DefaultBodyLimit::max(4 * 1024 * 1024)); // 4 MB
```
Individual endpoints that legitimately need larger bodies (e.g., bundle import) can override with `axum::extract::RequestBodyLimitLayer`.

**Progress as of 2026-05-21:**
- Added an Axum `DefaultBodyLimit` to the dashboard router.
- Default limit is 32 MiB to avoid breaking bundle import flows.
- Operators can override with `AGENT007_DASHBOARD_MAX_BODY_BYTES`.
- Remaining follow-up: add end-to-end oversized bundle/import regression tests and document the env var.

---

### Gap 10 — Skill registry fetches from unpinned `main` branch
**Severity: Medium**

**What the problem is:**
The skill registry endpoint fetches `https://raw.githubusercontent.com/danieldear/agent007/main/docs/registry.json` — always the current HEAD of the `main` branch. If the registry file or the repo were ever compromised, a malicious skill catalog could be served to users who click the Registry tab.

**Where it is in the code:**
`crates/web/src/api.rs`, `skill_registry_handler()` — the URL is hardcoded.

**Workaround until fixed:**
The registry is only fetched on user action (opening the Registry tab). It is not fetched automatically. Users should review skills before importing them.

**Work needed to close this gap:**
- Pin the registry URL to a specific commit SHA rather than `main` (update on each release)
- Or: add a `skill_registry_url` config option in `config.toml` so enterprise deployments can point to an internal, vetted catalog instead of the public one
- Add SHA256 content verification of the fetched JSON against a known-good hash

---

### Gap 11 — Extension installer has no signature verification
**Severity: Medium**

**What the problem is:**
The extension system (`crates/extensions/`) can install content from GitHub repos, npm packages, and URLs. There is path traversal protection (`sanitize_relative_path`), but there is no cryptographic signature check on installed content. A compromised GitHub repo or npm package would install without warning.

**Where it is in the code:**
`crates/web/src/extensions_api.rs` — `install_handler`. `crates/extensions/src/adapters/` — each adapter fetches and returns content without verifying signatures.

**Workaround until fixed:**
Only install extensions from repos/packages you personally control or have reviewed.

**Work needed to close this gap:**
- Add a `[extensions] allowlist = ["owner/repo", "npm-package-name"]` config option; reject installs not on the list
- For npm: pin to exact version + verify `npm pack` integrity hash
- For GitHub: require a specific tag or commit SHA, not a branch ref
- Consider adding a `--dry-run` flag to `install_extension` that shows the file list without writing to disk

---

### Gap 12 — Zone checker is opt-in, not enforced by default
**Severity: Medium**

**What the problem is:**
`ToolExecutor` is constructed with `zone_checker: None` by default. When no zone checker is configured, `check_zone()` returns `Ok(())` unconditionally — all file paths are effectively unrestricted. The zone system only protects when the user explicitly configures `[zones]` in `config.toml`.

**Where it is in the code:**
`crates/core/src/tool_executor.rs` — `pub fn new()` sets `zone_checker: None`. `check_zone()` returns early with `Ok(())` when `None`.

**Work needed to close this gap:**
- Apply a default `ZoneChecker` with sensible deny-by-default patterns even when `[zones]` is not configured:
  - Forbidden: `.env`, `*.pem`, `*.key`, `secrets/`, `keys/`, `.ssh/`, `.gnupg/`
  - Readonly: `.git/config`, `.agent007/config.toml`
- Document the default rules in `SECURITY.md` and `docs/configuration.md`
- Add an `--unrestricted` flag for power users who explicitly want to disable zone checks

---

### Gap 13 — No corporate identity / SSO integration
**Severity: Medium (for enterprise compliance)**

**What the problem is:**
- There is no SAML, OIDC, or LDAP integration
- API keys are per-developer environment variables with no central management or rotation
- Audit logs identify agents by name string, not by authenticated user identity
- There is no RBAC — everyone with access to the MCP server or dashboard has the same permissions

**Work needed to close this gap:**
This is a significant feature addition, not a single fix. Suggested incremental path:
1. Add an `authenticated_user` field to audit log entries (sourced from a configurable env var like `AGENT007_USER_ID` initially)
2. Add support for `AGENT007_DASHBOARD_TOKEN` tied to a named identity (see Gap 1)
3. Document a recommended pattern for secrets management (e.g., using `direnv` + `.envrc` per-project, or a secrets manager CLI like `op` or `vault` to inject keys at startup)
4. Longer-term: OIDC integration for the dashboard

---

### Gap 14 — No secrets vault integration
**Severity: Low (common for local developer tools)**

**What the problem is:**
API keys are read from environment variables. There is no integration with secrets managers (HashiCorp Vault, AWS Secrets Manager, 1Password `op`, etc.). This means key rotation requires developers to manually update their environment, and there is no centralized audit of key usage.

**Workaround:**
Use `direnv` + `.envrc` (gitignored) or a per-project `.env` loader that fetches secrets from a vault CLI at shell startup. agent007 reads `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` from the environment regardless of how they were set.

**Work needed to close this gap:**
- Document recommended patterns for vault-injected keys in `docs/configuration.md`
- Optionally: add a `[secrets]` config block that can run a command to retrieve a key value (e.g., `key_command = "op read op://vault/anthropic/key"`) — similar to how `pass` works with git credentials

---

### Gap 15 — No SBOM published with releases
**Severity: Low**

**What the problem is:**
No Software Bill of Materials is generated or published as part of the release process. Security teams doing SCA (software composition analysis) have to generate it themselves.

**Where it is in the code / release process:**
`scripts/install.sh` and the CI release pipeline — no SBOM generation step.

**Work needed to close this gap:**
- Add `cargo cyclonedx --format json --output sbom.cdx.json` to the release CI workflow
- Publish `sbom.cdx.json` as a GitHub Release artifact alongside the binary
- Add a note in `SECURITY.md` that the SBOM is available on the Releases page

**Can be done right now manually:**
```bash
cargo install cargo-cyclonedx
cargo cyclonedx --format json
# produces sbom.cdx.json in the workspace root
```

---

### Gap 16 — `agent007_git_commit` MCP tool has no mandatory approval gate
**Severity: Low**

**What the problem is:**
The `agent007_git_commit` MCP tool (`crates/cli/src/commands/serve.rs`) can stage and commit files to the local git repo when called by the AI editor. There is no hard-coded confirmation step. While the AI editor's own permission prompts provide one layer of protection, agent007 itself does not require a user confirmation before writing a commit.

**Note:** The workflow system has an `approval` step type that pauses for human review. But this is per-workflow configuration — ad-hoc calls to `agent007_git_commit` bypass it.

**Work needed to close this gap:**
- Add a `require_commit_approval = true` option to `[core]` in `config.toml` (default: `false` to preserve current behavior)
- When enabled: before executing a commit, emit a confirmation prompt to the MCP client via a tool response asking the user to approve the staged diff
- Document this option in `docs/configuration.md`

---

---

### Gap 17 — No dependency vulnerability scanning in CI
**Severity: Medium**

**What the problem is:**
`cargo audit` and related tools are not run in the CI pipeline. Known CVEs in transitive dependencies can silently enter the codebase and remain undetected until a manual check is run.

`cargo audit` only checks for known CVEs in `Cargo.lock` — it does not catch logic vulnerabilities in your own code. But it is the minimum automated bar.

**What each tool actually checks:**

| Tool | What it finds | What it misses |
|---|---|---|
| `cargo audit` | Known CVEs in `Cargo.lock` via RustSec DB | Your own code, logic bugs, OWASP issues |
| `cargo geiger` | Count and location of `unsafe {}` blocks | Whether the unsafe code is actually exploitable |
| `cargo deny` | CVEs + license violations + duplicate deps | Logic vulnerabilities |
| `cargo outdated` | Deps with newer versions available | Whether the newer version is safe |
| `trufflehog` / `gitleaks` | Secrets committed to git history | Secrets in env, runtime, or logs |
| SAST / manual review | Logic bugs, path traversal, SSRF, auth issues | Nothing — this is the only thing that finds them |

**Work needed to close this gap:**
Add to CI workflow (`.github/workflows/ci.yml`):
```yaml
- name: Security audit
  run: |
    cargo install cargo-audit --locked
    cargo audit

- name: Unsafe usage report
  run: |
    cargo install cargo-geiger --locked
    cargo geiger --all-features 2>&1 | tee geiger-report.txt

- name: Secret scan
  uses: trufflesecurity/trufflehog@main
  with:
    path: ./
    base: main
    head: HEAD
```

Run locally before any release:
```bash
cargo audit                   # CVEs in dependencies
cargo geiger                  # unsafe block inventory
cargo deny check              # advisories + licenses + duplicates
cargo outdated                # stale dependencies
trufflehog git . --since-commit HEAD~20   # secrets in recent history
gitleaks detect --source .    # broader secrets scan
```

---

## Ongoing audit process

Security is not a one-time checklist. These are the recurring activities needed to keep the gap count from growing:

**Every PR:**
- Reviewer checks any new HTTP fetch call for SSRF risk
- Reviewer checks any new filesystem write for path traversal risk
- Reviewer checks any new struct with `derive(Debug)` for credential exposure
- `cargo audit` runs in CI (Gap 17)

**Every release:**
- Run the full local toolchain above
- Update `Cargo.lock` (ensures `cargo audit` has current data)
- Review any new dependency added since last release
- Generate and publish SBOM (Gap 15)

**Before any enterprise pilot:**
- Gaps 1, 2, 3, 4, 5, 6, 7, 8 must be closed or have documented mitigations accepted in writing by the customer
- Gap 8 (LLM data exfiltration) requires either local-model support or a signed data processing agreement with each provider
- Conduct a focused manual review of all code paths from HTTP request to filesystem write and from HTTP request to outbound network call

---

## How to use this document

When a gap is resolved:
1. Move it from this file to a `## Closed Gaps` section with the version it was closed in
2. Update `SECURITY.md` to reflect the new behavior
3. Add a changelog entry in `CHANGELOG.md`

When a new gap is found:
1. Add it to the summary table with a severity and effort estimate
2. Write a detail section following the same format
3. Reference it from `SECURITY.md` if it affects the security architecture description
