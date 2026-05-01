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
- **Hosted workflow self-submitting steps**: every dispatched step now receives `session_id` + `step_id` injected into its prompt footer, allowing the step agent to call `agent007_workflow_submit_step` directly without the orchestrator holding context.
- **Lazy output fetching**: `agent007_workflow_get_output(session, key)` MCP tool lets step agents pull prior outputs on demand, eliminating full-output injection into the orchestrating context and reducing token bloat.
- **Heartbeat liveness**: `agent007_workflow_heartbeat(session, step, hint?)` writes a timestamped progress note to memory. Step prompts now require heartbeats every 3-5 minutes; steps silent for >10 min are flagged stale.
- **Staleness detection**: `workflow_status` and `workflow_next` compute `running_step_liveness` per in-flight step — shows last heartbeat hint, age, and `stale: true` if >10 min silent. A top-level `warnings` array surfaces stale steps to the host LLM.
- **Memory-backed step claims**: `workflow_next` writes a 2-hour TTL claim record per dispatched step; `workflow_submit_step` verifies the claim has not expired before accepting output.
- **Actual LLM token counts in skill executor**: `SkillExecutionMetrics` now carries `input_tokens` and `output_tokens` from the real API response; `run_skill_mcp` uses them instead of the `chars/4` heuristic when available.
- **`workflow_submit_step` tokens parameter**: optional `tokens: integer` field lets any hosted client (Codex, Cursor, etc.) report actual token usage inline without a separate `agent007_record_tokens` call.
- **Dashboard step liveness**: running workflow steps in the dashboard now show last heartbeat hint + relative age ("3m ago") with an animated pulse indicator; stale steps show a red `stale` badge and border.

### Changed

- README and release strategy docs aligned with the GitHub Releases + curl installer path.
- Regression test fixture in `crates/testing/src/regression.rs` updated for deterministic baseline threshold behavior.
- GitHub release workflow now builds Linux release artifacts only to reduce CI runtime; installer prints source-install guidance when macOS artifacts are unavailable.
- `WorkflowStepState` gains `last_heartbeat_at` and `last_heartbeat_hint` fields (backward-compatible via `#[serde(default)]`).
- Step prompt heartbeat instruction updated from "periodically" to "every 3-5 minutes; silence >10 min marks the step stale".

## [0.1.0] - 2026-04-27

### Added

- Initial public project baseline for `agent007` orchestration platform.
