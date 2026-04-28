# M5 — Shared Workspace

## 1. Concept

Shared Workspace extends the P2P collaboration layer (M4) to let trusted peers share three categories of data across agent007 instances: **memory**, **analysis artifacts**, and **tasks**. The goal is to eliminate redundant context-building across a team and make collective AI knowledge available to every peer in a session.

A developer on machine A who has already indexed a codebase and built up project memory should be able to offer that context to machine B — without either side having to rebuild it from scratch.

---

## 2. Why This Fits the Existing Architecture

The hard parts are already solved by M3.1 and M4:

| Already built | What Shared Workspace adds |
|---|---|
| Signed envelopes + identity | Payload types for memory, artifacts, tasks |
| Policy redaction filters | Per-entry `shareable` tag + scope-level opt-in |
| mDNS peer discovery | Shared workspace session negotiation |
| Request-response artifact pull | Memory pull + artifact subscription |
| `memory::MemoryStore` scoped KV + vector | `shared/` scope that syncs across peers |

Shared Workspace is "what goes inside the envelopes" — not a new transport layer.

---

## 3. Three Layers

### Layer 1 — Shared Memory
The most natural fit. Memory entries are already serializable KV + vector chunks stored in `~/.agent007/memory/`. A new `shared/` scope is added alongside `global/`, `project/`, `user/`, and `learning/`.

**How it works:**
- A peer explicitly tags an entry as shareable (`shareable: true` in the entry metadata) or shares a whole scope.
- Peers pull entries on demand using the existing request-response model from M4.
- The vector index for the `shared/` scope is populated on pull so RAG queries can reach team memory.

**Value:** A team lead who has written architectural decisions, gotchas, and runbook notes into project memory can make those available to every peer without copy-paste.

### Layer 2 — Shared Analysis Artifacts
Run artifacts (eval gate results, retrieval telemetry, routing recommendations, workflow outputs) are already structured, immutable JSON. Once a run completes it never changes — so there is no conflict problem.

**How it works:**
- Peers can subscribe to artifact streams by type (e.g., "all `code-review` workflow results from team peers this week").
- Artifacts are signed at the source and verified at ingest — same as M3.1 envelopes.
- Dashboard shows a "Team Artifacts" view alongside personal run history.

**Value:** If a teammate already ran a security audit on a shared library, peers can see the results without re-running the same expensive workflow.

### Layer 3 — Shared Tasks (Delegation)
The highest-value but highest-risk layer. A peer submits a task; another peer's agent007 instance executes it on their machine using their local model and memory context.

**How it works:**
- Task delegation requires explicit approval from the receiving peer before execution (extends the existing `workflow_approve` model to cross-peer tasks).
- The submitting peer sees task status via the existing run store API.
- Execution happens entirely within the receiving peer's zones and budget constraints — the delegating peer cannot override those.

**Risk controls:**
- Receiving peer must have delegating peer in their explicit trust allowlist.
- Task payloads are policy-filtered before execution (no arbitrary shell commands, only agent007 skill/workflow invocations).
- Budget governor enforces the receiving peer's own caps — not the submitter's.
- Full audit trail in both peers' run stores.

**Value:** A CI agent or a more powerful machine on the LAN can pick up expensive long-running tasks, freeing the developer's machine. Or a team lead can delegate a workflow run to a specialist agent persona running on a dedicated server.

---

## 4. Design Decisions

### 4.1 Conflict Resolution for Shared Memory
Shared memory entries can be written by multiple peers. Options:

| Strategy | Pros | Cons |
|---|---|---|
| Last-write-wins (LWW) with timestamp | Simple, no deps | Silent clobber on concurrent writes |
| LWW + conflict log | Simple, auditable | Still silently applies one version |
| Versioned entries (monotonic version counter) | Detectable conflicts | Requires version negotiation |
| CRDTs (e.g., Automerge) | True merge, no conflicts | Heavy dependency, complex |

**Recommendation for v1:** Versioned LWW — each entry carries a `(peer_id, version)` tuple. On conflict (same key, different peer, different version), both versions are preserved in a conflict log and the human is notified. CRDTs are deferred until there is demonstrated demand.

### 4.2 Opt-In Granularity
Three levels of opt-in, from coarsest to finest:

1. **Scope-level** — "share my entire `project/shared/` scope with trusted peers."
2. **Entry-level** — `shareable: true` flag on individual memory entries.
3. **Session-level** — "share everything for the duration of this workflow run."

All three should be supported. Default is nothing shared.

### 4.3 Privacy Boundary
Project memory frequently contains sensitive data: API keys, internal architecture decisions, customer data references. Controls:

- `shareable` defaults to `false` on all entries.
- The existing policy redaction layer is extended to strip or mask entries matching configured patterns before they leave the machine.
- Raw LLM prompt/output entries are excluded from sharing by default (same rule as M3.1 artifact bundles).
- A pre-share dry-run command shows exactly what would be shared before any data leaves.

### 4.4 Ownership Model (v1)
Each peer owns the entries they wrote. Another peer can pull a copy but cannot modify the original on the owning peer's machine. A peer can revoke sharing of an entry at any time; revocation propagates as a tombstone envelope to known peers.

Full collaborative editing (any peer can write to a shared entry) is deferred to a later phase once the simpler pull model is validated.

---

## 5. Phases

### Phase 1 — Shared Memory Pull (read-only)
- `shared/` scope in `memory::MemoryStore`.
- `shareable` entry metadata flag.
- Pull API: peer A requests entries from peer B by key or tag.
- Pre-share dry-run command.
- Dashboard: "Shared Memory" panel showing entries pulled from peers.
- Policy redaction extended to memory entries.

### Phase 2 — Shared Analysis Artifacts
- Artifact subscription API: subscribe to artifact types from specific peers.
- Dashboard "Team Artifacts" view.
- Artifact feed integrated into run-detail context (e.g., "a peer ran code-review on this file 2 days ago — here are their findings").

### Phase 3 — Task Delegation
- Cross-peer task submission + approval flow.
- Task status propagation back to submitting peer.
- Zones + budget enforcement on receiving side.
- Audit trail in both run stores.
- Revocation: delegating peer can cancel a pending task before execution starts.

---

## 6. Dependencies

| Dependency | Status |
|---|---|
| M3.1 — signed envelopes + policy filters | ✅ Complete |
| M4 — mDNS peer discovery | 📋 Planned |
| M4 — request-response artifact pull | 📋 Planned |
| M4 — peer trust/allowlist controls | 📋 Planned |

**M5 Phase 1 (shared memory pull) can begin as soon as M4 request-response is in place.**
M5 Phase 3 (task delegation) should only start after M4 trust controls are hardened and validated.

---

## 7. Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Sensitive data leaks via shared memory | `shareable` opt-in default + pre-share dry-run + policy redaction |
| Conflict corruption of shared memory | Versioned LWW + conflict log + human notification |
| Remote code execution via task delegation | Approval gate + policy-filtered payloads + receiving peer's own zones/budget |
| Stale shared entries polluting RAG context | TTL metadata on shared entries; peers can set max-age on pulled entries |
| Trust compromise (allowlist bypass) | M4 identity + signing layer; unknown peers rejected at ingest |

---

## 8. Success Metrics

1. Reduction in duplicate workflow runs across a team on the same codebase.
2. Measurable RAG context improvement when shared team memory is available vs. not.
3. Zero unauthorized data-share incidents in controlled pilot.
4. Task delegation approval flow completes in under 30 seconds for simple tasks.

---

## 9. Out of Scope

1. Internet-wide shared workspaces (LAN-first, always).
2. Centralized server for workspace sync (fully peer-to-peer).
3. Automatic conflict merging via CRDTs (deferred until validated demand).
4. Real-time collaborative editing of memory entries (deferred to a later phase).
5. Sharing raw LLM prompts or outputs (excluded by default, policy-controlled).
