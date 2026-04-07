# ADR-006: Synchronous Hook Execution

**Date:** 2026-04-07  
**Status:** Accepted  
**Deciders:** agent007 core team

## Context

Hooks are user-defined shell commands that fire on agent007 lifecycle events (e.g., `pre_skill`, `post_skill`, `pre_workflow_step`, `post_workflow_step`). They allow users to extend agent007's behavior without modifying the core — common use cases include writing context to memory before a skill runs, logging executions to an audit file, or triggering a notification when a workflow completes.

Two execution models were evaluated:

1. **Synchronous**: `HookExecutor::fire()` spawns the subprocess and `await`s its completion before the MCP handler returns
2. **Asynchronous**: The subprocess is spawned and the handler continues immediately (fire-and-forget, or queued for later)

## Decision

Hooks are executed **synchronously**. `HookExecutor::fire()` spawns the hook command as a subprocess, awaits its completion, captures its exit code and output, and only then returns control to the calling MCP handler. The hook runs inside the async tokio runtime via `tokio::process::Command`.

## Rationale

- **Causal ordering**: The most valuable hooks are those that *prepare* context before an operation runs. A `pre_skill` hook that writes project state to memory must complete before the skill prompt is constructed — async fire-and-forget cannot guarantee this ordering.
- **Simplicity**: Synchronous execution requires no queue, no completion tracking, and no reconciliation between hook state and handler state. The implementation is a straightforward `spawn().wait()`.
- **No orphaned processes**: Fire-and-forget hooks can outlive the MCP session. If the editor is closed while a background hook is running, the subprocess is orphaned. Synchronous execution ties the subprocess lifetime to the MCP request.
- **Error visibility**: A synchronous hook that exits non-zero can propagate a warning or error back to the MCP caller in the same response. An async hook's failure is invisible to the user unless an external logging mechanism is in place.
- **Hook commands should be fast by design**: Hooks are documented as lightweight shell commands — writing a memory entry, appending to a log file, sending a local notification. Operations in this class complete in milliseconds. The synchronous model is appropriate for this use case.

## Alternatives Considered

| Alternative | Reason Not Chosen |
|-------------|------------------|
| **Async fire-and-forget** | Simpler to implement (no `await`), but non-deterministic ordering breaks the `pre_skill` use case. Orphaned processes and invisible failures are additional downsides |
| **Dedicated event queue with worker thread** | A bounded async channel where hooks are queued and a background worker processes them in order. Provides ordering guarantees without blocking the handler. Rejected as over-engineered — the additional complexity (queue, worker lifecycle, backpressure) is not justified for shell commands that should complete in under a second |
| **Async with timeout** | Spawn the hook, await it with a hard timeout (e.g., 5 s), continue on timeout. Provides a safety valve but adds complexity and still risks partial execution (hook started but not finished). Decided that the timeout responsibility belongs in the user's hook script, not the executor |
| **Optional sync/async per hook** | Allow users to annotate hooks as `async: true`. Adds a configuration dimension and two code paths to maintain. Rejected for simplicity; all hooks are synchronous. |

## Consequences

### Positive

- `pre_skill` hooks reliably complete before the skill prompt is assembled — the primary motivating use case works correctly
- Hook failures are surfaced immediately in the MCP response; users see errors rather than silent failures
- No background processes outlive the MCP session
- Implementation is simple and auditable — the hook execution path is a thin wrapper around `tokio::process::Command`

### Negative / Tradeoffs

- **Blocking risk**: A hook command that hangs (e.g., a `curl` call to an unresponsive server, a `git` operation on a large repo, a misconfigured script with an infinite loop) will block the MCP handler thread for the duration of the hang. The editor will appear unresponsive until the hook times out or is killed.
- **User responsibility for timeouts**: Users must write their hook scripts defensively — use `timeout 5 <command>` in shell hooks, avoid operations with unbounded duration. This requirement is documented but not enforced by the executor.
- **Sequential multi-hook execution**: If multiple hooks are registered for the same event, they execute sequentially. There is no parallel hook execution. For the typical 1–3 hooks per event this is negligible, but it could add latency if a user registers many hooks.
- **No hook output in MCP result**: Hook stdout/stderr is captured and logged internally but not currently forwarded to the MCP caller as part of the tool response. Debugging a misbehaving hook requires checking agent007 server logs.

## Related ADRs

- ADR-002 — MCP stdio transport (hooks fire within MCP tool handler calls)
- ADR-005 — Skills as Markdown with frontmatter (pre/post skill hooks are the primary hook use case)
