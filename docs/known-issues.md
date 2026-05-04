# Known Issues

This page tracks currently known product bugs or behavior gaps that are
important for users to understand before relying on a feature.

## Workflow approval ownership and dashboard resume behavior

**Status:** Intentional current behavior  
**Observed:** April 11, 2026  
**Area:** Hosted workflows, web dashboard, approval UX

### Current behavior

Workflow approval is surfaced to the client that initiated the workflow run
when that client is available to own the conversation.

Update (April 28, 2026):
- Dashboard `Running`/`Active Agents` metrics now count only actively executing
  runs.
- Approval-paused runs are tracked separately as `awaiting approval`.

Examples:
- A workflow started from Codex should surface the approval back to Codex.
- A workflow started from Claude Code should surface the approval back to
  Claude Code.
- The web dashboard is read-only for externally initiated approval-gated runs.
- Dashboard-owned standalone runs can still be resumed from the dashboard.

### Practical guidance

- Use the initiating client as the approval and continuation surface for
  externally started workflows.
- Use the dashboard for monitoring, diagnostics, and standalone dashboard runs.
- If a run originated in the dashboard's standalone runtime, `Resume Workflow`
  remains available there.
- If old paused approvals accumulate, clean them with:
  `POST /api/runs/cleanup-awaiting` (for example, `older_than_hours: 168`).

## Runtime mode fallback between standalone and hosted-MCP is incomplete

**Status:** Known behavior gap  
**Observed:** April 14, 2026  
**Area:** Runtime selection, standalone vs hosted execution

### Current behavior

If project config still declares an Ollama provider, `agent007` may classify the
runtime as standalone/local even when the Ollama service is unavailable.

Update (April 28, 2026):
- Runtime provider selection now performs a fast Ollama health check.
- If Ollama is unreachable and no cloud provider is available, runtime mode now
  degrades to `hosted-mcp` instead of staying in broken standalone mode.

Examples:
- `agent007 run` can fail or hang waiting on the local Ollama endpoint.
- Skill execution can fail even though hosted MCP would have been a valid path.
- Direct standalone commands and dashboard task execution do not automatically
  borrow the currently connected host LLM from Codex, Claude Code, Cursor, or
  another MCP host.
- In hosted-MCP mode, direct CLI `run` is not equivalent to \"run this through
  the host LLM\"; it remains a separate execution path.

### Practical guidance

- If Ollama is down and you want to continue via Codex, Claude Code, Cursor, or
  another MCP host, disable the project-local Ollama provider and restart
  `agent007 serve`.
- Treat hosted-MCP and standalone/local runtime as separate operating modes for
  now.
- Use MCP tools from the initiating host client when you want hosted execution;
  do not expect `agent007 run` or direct dashboard task execution to implicitly
  proxy through that host.
- Future improvements should:
  - detect unreachable Ollama and automatically fallback to hosted-MCP when a
    host client is available
  - expose clearer degraded-state/runtime-mode messaging
  - reduce confusion between hosted execution, standalone execution, and mock
    fallback behavior

## Hosted workflow steps go silent with no liveness signal — resolved

**Status:** Fixed (2026-05-01)  
**Area:** Hosted workflows, staleness detection

Previously, running steps provided no liveness signal. If a background step silently stalled, the workflow sat in `awaiting-outputs` indefinitely with no indication of whether work was in progress or the step had crashed.

**Resolution:** `workflow_heartbeat` now persists heartbeat data into `workflow-state.json` as well as memory. `workflow_status` computes `running_step_liveness` per in-flight step with hint + age. Steps silent for >10 min are flagged `stale: true` in both the MCP response and the dashboard (red badge, stale warning). Step prompts explicitly require heartbeats every 3-5 minutes.

---

## Token counts show estimated values instead of actual LLM usage — resolved

**Status:** Fixed (2026-05-01)  
**Area:** Token metrics, dashboard

In standalone mode, `run_skill_mcp` used `output.len() / 4` as the token count regardless of whether actual counts were available from the LLM API. Claude's API response already populated `input_tokens` and `output_tokens` but they were dropped by the skill executor.

**Resolution:** `SkillExecutionMetrics` now carries actual `input_tokens` / `output_tokens` from the API response. `run_skill_mcp` uses the real sum when available; the char estimate is a fallback only for providers that don't return usage. `workflow_submit_step` accepts an optional `tokens` parameter so hosted clients (Codex, Cursor) can report actual usage inline.

---

## Hosted `record_tokens` memory capture can appear sparse if output is omitted

**Status:** Partially improved  
**Observed:** April 28, 2026  
**Area:** Hosted MCP run recording, project memory

### Current behavior

`agent007_record_tokens` now writes structured memory records in project scope:
- `run_records:<run_id>`
- `<kind>_runs:<run_id>`
- `<kind>_last`

When `output` is missing but `output.txt` already exists, that stored output is
now reused for memory persistence and run preview.

Remaining gap:
- If the host never provides output and no stored output exists, the run can
  still finalize with a generic preview (`completed`) and limited memory value.

### Practical guidance

- Always send `output` in `agent007_record_tokens` for high-quality memory.
- Keep hosted responses concise but complete enough to be reused as memory.

---

## Memory visibility and learning liveness were opaque — resolved

**Status:** Fixed (2026-05-03)  
**Area:** Memory observability, runtime learning workers

Previously, users could not easily see what was stored in memory per scope, and learning optimization behavior could appear inconsistent across command paths.

**Resolution:**
1. Added `GET /api/memory/{scope}/stats` to expose:
   - total key count
   - semantic/procedural/episodic counts
   - average confidence
   - learning-scope skill coverage
2. Dashboard memory view now surfaces these stats.
3. Learning runtime workers are now spawned in both `run` and `serve` flows so optimizer/insight loops stay active.
4. Memory key compatibility was hardened for mixed key styles (`:` and `/`) with legacy fallback and migration support.
