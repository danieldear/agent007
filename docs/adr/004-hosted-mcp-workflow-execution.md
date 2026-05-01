# ADR-004: Hosted-MCP Workflow Execution Mode

**Date:** 2026-04-07  
**Status:** Accepted  
**Deciders:** agent007 core team

## Context

Workflows in agent007 are multi-step pipelines. Each step contains a prompt that must be executed by a large language model (LLM). The challenge is architectural: the agent007 MCP server itself cannot make LLM calls because:

1. It does not hold the user's LLM API credentials — those live in the editor/host environment
2. Making outbound LLM calls from the server would create a recursive loop (the MCP server calling an LLM that is itself driving the MCP server)
3. Forwarding credentials from the host to the server creates a significant security surface
4. The host LLM already has access to its own full tool set (file system, shell, other MCP servers) and can apply those tools during step execution — this capability should not be lost

A workflow execution model is needed that leverages the host LLM's existing connection without requiring the server to make independent LLM calls.

## Decision

Implement **Hosted-MCP workflow execution**: the `WorkflowEngine` manages workflow state server-side, but delegates actual step execution to the host LLM through a structured request/response loop over the existing MCP connection.

The protocol:

1. Host calls `agent007_workflow_start(name, task)` → server creates a session, returns `session_id`
2. Host calls `agent007_workflow_next(session_id)` → server returns the next batch of ready steps (steps whose `depends_on` are all satisfied), each with a fully-rendered prompt. Each step includes `session_id` and `step_id` injected into the prompt footer so the step agent can self-submit.
3. Host executes each step prompt using its own LLM context and tools
4. **Self-submit**: the step agent calls `agent007_workflow_submit_step(session_id, step_id, output[, tokens])` directly — the orchestrator does not need to relay the output. An optional `tokens` integer field accepts actual token usage for accurate dashboard metrics.
5. The server verifies a memory-backed step claim (2-hour TTL) before accepting submission, preventing replay of stale or duplicate outputs.
6. Repeat until all steps are complete or a human approval gate is reached

Additional tools available to step agents during execution:

- `agent007_workflow_get_output(session, key)` — fetch a prior step's output on demand without injecting it into the orchestrating context (avoids token bloat)
- `agent007_workflow_heartbeat(session, step, hint?)` — write a timestamped liveness note every 3-5 minutes; steps silent for >10 min are flagged stale in `workflow_status` responses and the dashboard
- `agent007_workflow_status(session)` — inspect current session state, per-step liveness, and any stale warnings

Approval gates pause execution and return a structured response requiring a `agent007_workflow_approve(session_id, decision)` call before proceeding.

## Rationale

- **No credential forwarding**: The host LLM executes step prompts using its own context. The server never needs an API key.
- **Full host tool access during steps**: When the host LLM executes a step, it has access to all its tools — file system, shell, other MCP servers, web search. A server-side LLM call would be isolated to whatever tools were statically configured in the server.
- **Observable, resumable sessions**: Each step's output is stored server-side with a `run_id`. Sessions can be inspected (`agent007_workflow_status`), resumed (`agent007_workflow_resume`), and queried in the run history. This would be impossible with fire-and-forget execution.
- **Approval gates integrate naturally**: Because the server controls the step dispatch loop, it can inject approval checkpoints that pause dispatch until a human explicitly approves or edits the output. This is a first-class feature, not a hack.
- **Parallel step dispatch**: Steps without mutual dependencies are returned together in the same `workflow_next` response. The host can execute them concurrently.

## Alternatives Considered

| Alternative | Reason Not Chosen |
|-------------|------------------|
| **Server-side LLM calls** | Would require an `OPENAI_API_KEY` or equivalent in the server config. Adds credential management complexity, increases the attack surface, and loses access to the host's full tool set during step execution |
| **Client-side orchestration via `agent007_run`** | The host could call `agent007_run` for each step and manually track dependencies. This loses workflow-level state management, approval gates, session history, and parallel dispatch — essentially reimplementing the WorkflowEngine on the host side |
| **Background agent spawning** | The server could spawn independent agent processes per step. Rejected because it requires either credential injection (same problem as server-side calls) or inter-process communication complexity |
| **Event-driven via webhooks** | Server posts step results to a webhook URL. Rejected for the primary integration because it requires the host to run an HTTP server and handle async delivery — too much infrastructure for a local developer tool |

## Consequences

### Positive

- Works with any MCP-compatible host that can execute prompts — no agent007-specific host code required
- Session state (step outputs, run IDs, approval decisions) is durably stored server-side and survives editor restarts
- The run history (`agent007_run_history`, `agent007_run_show`) provides a full audit trail of workflow executions
- Parallel steps are explicitly modeled — the host receives a batch and can choose to execute sequentially or concurrently

### Negative / Tradeoffs

- **Active host participation required**: Workflow execution is not autonomous. If the host LLM drops its context or the editor is closed mid-workflow, execution pauses until `workflow_next` is called again. Sessions do not self-advance.
- **Latency**: Each step requires at least two round-trip MCP calls (`workflow_next` + `workflow_submit_step`) in addition to the LLM call itself. For workflows with many sequential steps, this adds perceptible overhead.
- **Host fidelity variance**: Step outputs depend on the host LLM's interpretation of the step prompt. The same workflow may produce different results across different host models or sessions.
- **`run_id` tracking**: The host must track `run_id` values to record actual token usage via `agent007_record_tokens`. This bookkeeping is easy to skip, leading to inaccurate dashboard metrics. Mitigated by the optional `tokens` parameter on `workflow_submit_step` for hosted clients that can report usage inline.

## Related ADRs

- ADR-001 — Rust as implementation language (WorkflowEngine is an async Rust state machine)
- ADR-002 — MCP stdio transport (the hosted-MCP pattern relies on the existing MCP connection)
- ADR-003 — YAML for workflow definitions (the WorkflowEngine loads and validates these files)
