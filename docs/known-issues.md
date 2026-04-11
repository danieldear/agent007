# Known Issues

This page tracks currently known product bugs or behavior gaps that are
important for users to understand before relying on a feature.

## Workflow approval and dashboard resume semantics

**Status:** Open  
**Observed:** April 11, 2026  
**Area:** Hosted workflows, web dashboard, approval UX

### Intended behavior

Workflow approval is intended to happen in the client that initiated the
workflow run.

Examples:
- A workflow started from Codex should surface the approval back to Codex.
- A workflow started from Claude Code should surface the approval back to
  Claude Code.
- The web dashboard should act as an observer/monitoring surface unless the
  workflow was explicitly started from the dashboard's own standalone runtime.

### Current bug

The web dashboard can present a `Resume Workflow` action for approval-gated runs
even when the workflow was not conceptually owned by the dashboard session.

In the problematic path:

```text
workflow pauses for approval
-> approval is recorded
-> dashboard offers Resume Workflow
-> resume creates a continuation run in the web runtime
-> original paused run can still appear resumable
-> repeated clicks may create additional continuation runs
```

This is a UX and execution-boundary bug:
- the approval/continuation loop is being surfaced in the wrong place
- the dashboard resume model can fork continuation runs instead of feeling like
  a single in-place continuation

### Practical guidance

Until this is redesigned:
- Treat the dashboard as a monitoring surface for hosted workflows.
- Prefer completing approval/continuation in the client that started the run.
- Avoid using the dashboard `Resume Workflow` action as the primary control path
  for workflows that originated in Codex, Claude Code, Cursor, Copilot, or Zed.

### Fix direction

The likely product-level correction is:

```text
workflow owner = initiating client/session
dashboard = visibility + diagnostics
approval prompts = returned to initiating client
resume/continue = performed by initiating client or host runtime
```

This keeps approval ownership aligned with the conversation context that
actually initiated the workflow.
