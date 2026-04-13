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
