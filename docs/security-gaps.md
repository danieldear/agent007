# Security Gaps — Hardening Roadmap

This document lists known security gaps in the current codebase, their severity for enterprise/office deployment, and what work is needed to close each one. It is maintained by the project team. Gaps are closed by removing them from this list and updating `SECURITY.md` accordingly.

**Last updated:** 2026-05-08  
**Current version:** 0.2.0

---

## Summary Table

| # | Gap | Severity | Effort | Status |
|---|---|---|---|---|
| 1 | Web dashboard has no authentication | High | Medium | Open |
| 2 | Web server binds `0.0.0.0` by default | High | Low | Open |
| 3 | Skill registry fetches from unpinned `main` branch | Medium | Low | Open |
| 4 | Extension installer has no signature verification | Medium | High | Open |
| 5 | Zone checker is opt-in, not enforced by default | Medium | Medium | Open |
| 6 | No corporate identity / SSO integration | Medium | High | Open |
| 7 | No secrets vault integration | Low | High | Open |
| 8 | No SBOM published with releases | Low | Low | Open |
| 9 | `agent007_git_commit` has no mandatory approval gate | Low | Low | Open |

---

## Gap Details

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
- Add a configurable static bearer token (set via `AGENT007_DASHBOARD_TOKEN` env var or `config.toml`)
- Apply the token check as an Axum middleware layer before all `/api/*` routes
- Optionally: add an `--auth-token <token>` CLI flag to `serve` command
- Document the token requirement in `SECURITY.md` and `docs/configuration.md`

---

### Gap 2 — Web server binds `0.0.0.0` by default
**Severity: High** (closely related to Gap 1)

**What the problem is:**
`TcpListener::bind("0.0.0.0:{port}")` binds all network interfaces. On a developer laptop on a corporate network, this means the dashboard is reachable from any other machine on the same subnet.

**Where it is in the code:**
`crates/web/src/server.rs`, lines 270, 315 — `format!("0.0.0.0:{port}")`.

**Workaround until fixed:**
Use `--no-dashboard` or firewall the port.

**Work needed to close this gap:**
- Change the default bind address to `127.0.0.1` (localhost only)
- Add a `--bind <addr>` CLI flag to `serve` and `serve-web` commands so operators who need LAN/remote access can opt in explicitly
- Update `docs/configuration.md` with the flag documentation

---

### Gap 3 — Skill registry fetches from unpinned `main` branch
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

### Gap 4 — Extension installer has no signature verification
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

### Gap 5 — Zone checker is opt-in, not enforced by default
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

### Gap 6 — No corporate identity / SSO integration
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

### Gap 7 — No secrets vault integration
**Severity: Low (common for local developer tools)**

**What the problem is:**
API keys are read from environment variables. There is no integration with secrets managers (HashiCorp Vault, AWS Secrets Manager, 1Password `op`, etc.). This means key rotation requires developers to manually update their environment, and there is no centralized audit of key usage.

**Workaround:**
Use `direnv` + `.envrc` (gitignored) or a per-project `.env` loader that fetches secrets from a vault CLI at shell startup. agent007 reads `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` from the environment regardless of how they were set.

**Work needed to close this gap:**
- Document recommended patterns for vault-injected keys in `docs/configuration.md`
- Optionally: add a `[secrets]` config block that can run a command to retrieve a key value (e.g., `key_command = "op read op://vault/anthropic/key"`) — similar to how `pass` works with git credentials

---

### Gap 8 — No SBOM published with releases
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

### Gap 9 — `agent007_git_commit` MCP tool has no mandatory approval gate
**Severity: Low**

**What the problem is:**
The `agent007_git_commit` MCP tool (`crates/cli/src/commands/serve.rs`) can stage and commit files to the local git repo when called by the AI editor. There is no hard-coded confirmation step. While the AI editor's own permission prompts provide one layer of protection, agent007 itself does not require a user confirmation before writing a commit.

**Note:** The workflow system has an `approval` step type that pauses for human review. But this is per-workflow configuration — ad-hoc calls to `agent007_git_commit` bypass it.

**Work needed to close this gap:**
- Add a `require_commit_approval = true` option to `[core]` in `config.toml` (default: `false` to preserve current behavior)
- When enabled: before executing a commit, emit a confirmation prompt to the MCP client via a tool response asking the user to approve the staged diff
- Document this option in `docs/configuration.md`

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
