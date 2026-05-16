# M4 — Runtime Sessions, Agent Collaboration, and TUI Usability

## Goal
Strengthen agent007 as a long-lived orchestration runtime by improving session persistence, agent-to-agent coordination, memory flow, browser/provider UX, compact runtime visibility, and terminal usability without changing the product into a terminal-first clone of jcode.

## Non-Goals
1. Rebuild agent007 around a custom terminal renderer.
2. Replace the web dashboard as the primary control surface.
3. Chase full parity with jcode swarm/runtime features.
4. Introduce speculative distributed execution before single-host session semantics are solid.

## Why This Milestone Exists
agent007 already has:
- a hosted workflow engine
- MCP server and dashboard
- ETR built-ins for deterministic work
- memory, workflows, and personas

What it lacks is a stronger **runtime layer**:
- better long-lived session semantics
- explicit agent messaging
- tighter memory lifecycle
- simpler provider/browser setup
- compact live status in both web and terminal views
- a TUI that is usable for daily runtime monitoring

## Workstreams

### W1 — Session Server Model
Add a clearer long-lived session model around the existing MCP/runtime surfaces.

**Scope**
- candidate crates/files:
  - `/Users/neo/workspace/agent007/crates/cli/src/commands/serve.rs`
  - `/Users/neo/workspace/agent007/crates/workflows/`
  - `/Users/neo/workspace/agent007/crates/web/src/api.rs`
  - `/Users/neo/workspace/agent007/crates/web/frontend/src/views/`

**Deliverables**
1. Session inventory API:
   - active sessions
   - last heartbeat
   - owning workflow/run
   - current status
2. Session resume semantics:
   - reconnect without losing step state
   - explicit stale/orphaned session handling
3. Session lifecycle rules:
   - created
   - active
   - idle
   - awaiting approval
   - stale
   - completed

**Acceptance**
- sessions survive routine client reconnects
- stale sessions are visible and recoverable
- hosted workflow state is inspectable without digging through raw files

### W2 — Agent-to-Agent Messaging
Make collaboration explicit instead of implicit via only workflow state.

**Scope**
- candidate crates/files:
  - `/Users/neo/workspace/agent007/crates/workflows/`
  - `/Users/neo/workspace/agent007/crates/core/`
  - `/Users/neo/workspace/agent007/crates/web/src/api.rs`
  - dashboard session/run detail views

**Deliverables**
1. Internal message envelope:
   - from
   - to
   - session/run
   - message kind
   - payload
   - timestamp
2. Message classes:
   - request
   - handoff
   - progress note
   - warning/blocker
   - result summary
3. Compact UI surface showing:
   - last N messages
   - blocked handoffs
   - unacknowledged requests

**Acceptance**
- workflow steps can exchange structured messages
- users can inspect handoffs in dashboard/TUI
- blocked coordination is visible without opening raw traces

### W3 — Memory Architecture Improvements
Improve how useful memory is captured, compacted, and reused.

**Scope**
- candidate crates/files:
  - `/Users/neo/workspace/agent007/crates/memory/`
  - `/Users/neo/workspace/agent007/crates/learning/`
  - `/Users/neo/workspace/agent007/crates/web/src/api.rs`
  - memory views in frontend

**Deliverables**
1. Memory classes:
   - explicit saved note
   - run artifact summary
   - reusable skill/workflow output
   - ephemeral session memory
2. Save-path rules:
   - what is auto-recorded
   - what requires explicit promotion
3. Compact retrieval summaries:
   - high-signal snippets
   - source attribution
   - freshness/age markers

**Acceptance**
- repeated sessions reuse relevant prior outcomes with lower context bloat
- users can tell why a memory item exists
- memory retrieval is inspectable and suppressible

### W4 — Browser / Provider UX
Reduce friction for setup and day-to-day usage.

**Scope**
- candidate crates/files:
  - `/Users/neo/workspace/agent007/crates/web/frontend/src/views/`
  - `/Users/neo/workspace/agent007/crates/web/src/api.rs`
  - provider/browser config surfaces

**Operating model**
- provider onboarding is **dashboard-first**
- CLI/env/config setup remains supported
- dashboard actions should write or validate the same underlying config surface instead of inventing a second runtime model
- hosted-MCP mode remains valid even when no standalone provider is configured

**Deliverables**
1. Provider status card:
   - configured / missing / degraded
   - endpoint in use
   - auth state
   - runtime mode impact (hosted-mcp / standalone / local-ollama)
2. Browser capability card:
   - available backends
   - health
   - quick test action
3. Better setup paths:
   - dashboard onboarding/wizard for supported provider types
   - concise validation messages
   - direct fix hints
   - no silent failures
4. Provider classes to support incrementally:
   - env-backed API providers already in repo (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`)
   - local/self-hosted endpoints (`[models.ollama]`)
   - OpenAI-compatible endpoint configuration
   - later: selected OAuth/account-backed providers if worth owning
5. Compatibility rules:
   - preserve manual `config.toml` editing
   - preserve env var overrides
   - never require dashboard usage for automation/headless setups

**Acceptance**
- users can set up or validate a provider from the dashboard
- users can tell why a provider/browser feature is unavailable
- setup failures are actionable
- quick validation works from dashboard
- manual config/env workflows continue to work unchanged

### W5 — Compact Runtime Visibility
Expose runtime state in a compact, operator-friendly way.

**Scope**
- web dashboard + TUI surfaces
- ETR/runtime status helpers

**Deliverables**
1. Compact session cards:
   - workflow/run
   - phase
   - progress
   - last tool
   - pending approval
   - last error
2. Runtime summary endpoints for:
   - active sessions
   - blocked sessions
   - stale sessions
   - recent failures
3. ETR-backed inspection where possible for low-noise summaries

**Acceptance**
- users can understand runtime health in one screen
- noisy raw JSON is not required for normal triage
- terminal and dashboard surfaces show aligned status

### W6 — TUI Usability
Make the terminal experience genuinely usable for monitoring and control.

**Scope**
- candidate CLI/TUI surfaces in:
  - `/Users/neo/workspace/agent007/crates/cli/`
  - dashboard parity for status concepts

**Deliverables**
1. TUI views:
   - sessions list
   - run detail
   - approvals queue
   - recent errors
2. Keyboard actions:
   - inspect
   - retry
   - approve/deny
   - copy summary
3. Compact layout rules:
   - no giant JSON dumps
   - stable widths
   - truncation with drill-in

**Acceptance**
- TUI is usable for day-to-day monitoring
- approvals and failures can be handled without leaving terminal
- status is readable on normal laptop terminal sizes

### W7 — Mock Viewer and Diagram Preview
Make generated visual/design artifacts first-class in the dashboard so users can review them without leaving agent007.

**Scope**
- candidate crates/files:
  - `/Users/neo/workspace/agent007/crates/web/src/api.rs`
  - `/Users/neo/workspace/agent007/crates/web/frontend/src/views/`
  - `/Users/neo/workspace/agent007/crates/web/frontend/src/components/`
  - candidate artifact-serving helpers in web/runtime crates

**Why this belongs here**
- the dashboard already exists
- runs/workflows already produce artifacts
- agent007 increasingly handles UI/UX, architecture, and design-adjacent tasks
- users should be able to render and review outputs directly instead of manually opening files elsewhere

**Primary use cases**
1. design viewing in the web dashboard
2. rendering generated UI/UX mock outputs during tasks
3. rendering flow/mermaid/architecture diagrams

**Deliverables**
1. Viewer surface in dashboard:
   - modal, drawer, or dedicated artifact pane
   - linked from run/workflow outputs
2. Render modes:
   - Mermaid text → rendered diagram
   - static image preview (PNG/SVG/WebP)
   - HTML/CSS mock preview in a sandboxed iframe
   - raw source fallback
3. Artifact metadata:
   - type
   - size
   - source run/session
   - renderability flags
4. Review actions:
   - open
   - copy raw source
   - download artifact
   - open related run/session context

**Explicit v1 boundaries**
- not a full design editor
- not a Figma replacement
- not an arbitrary JS app host
- not browser automation embedded into the viewer
- not unrestricted execution of generated code

**Acceptance**
- users can render Mermaid outputs directly in dashboard
- users can preview generated mock/image artifacts without leaving agent007
- HTML/CSS previews are sandboxed
- non-renderable artifacts degrade cleanly to raw/source view
- viewer integrates with workflow/run outputs instead of being a disconnected file browser

## Recommended Order
1. **W1 Session Server Model**
2. **W5 Compact Runtime Visibility**
3. **W6 TUI Usability**
4. **W7 Mock Viewer and Diagram Preview**
5. **W2 Agent-to-Agent Messaging**
6. **W4 Browser / Provider UX**
7. **W3 Memory Architecture Improvements**

## Suggested PR Slices

### Slice A — Session Inventory and Lifecycle
- add session list/status API
- add stale/orphan detection
- add dashboard session summary view

### Slice B — Compact Runtime Summary
- add compact status endpoints/helpers
- add dashboard runtime cards
- align summary shape with terminal output

### Slice C — First Usable TUI
- sessions list
- run detail
- approval queue

### Slice D — Agent Messaging Core
- message envelope
- persistence
- message inspection UI

### Slice E — Provider / Browser UX
- provider health/status
- browser health/status
- setup validation and fix hints

### Slice F — Memory Lifecycle
- memory classes
- promotion rules
- retrieval summary visibility

### Slice G — Mock Viewer and Diagram Preview
- artifact viewer panel/modal
- Mermaid renderer
- static image preview
- sandboxed HTML/CSS mock preview
- raw source fallback
- link viewer from workflow/run outputs

## Definition of Done
1. Long-lived sessions are visible, resumable, and recoverable.
2. Runtime status is compact in both dashboard and terminal views.
3. TUI supports normal operator tasks without raw JSON dependence.
4. Agent handoffs are inspectable.
5. Provider/browser failures are diagnosable from UI.
6. Memory reuse is more transparent and less noisy.
7. Generated visual artifacts can be reviewed directly in dashboard.
