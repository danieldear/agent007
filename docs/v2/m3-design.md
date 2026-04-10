# Design — V2 M3 Collaboration + Adaptive Foundation

## 1. Design Goals
1. Add collaboration and adaptive intelligence foundations without disrupting baseline behavior.
2. Keep all new behavior feature-gated and auditable.
3. Provide deterministic fallback when any new subsystem is unavailable.

## 2. Logical Layers
1. Execution Layer: existing runner/hosted execution.
2. Collaboration Layer: filter -> redact -> sign -> sync.
3. Evaluation Layer: score -> baseline compare -> gate decision.
4. Recommendation Layer: shadow-mode route suggestions with confidence.

## 3. Component Specs
1. Collaboration Service
- Inputs: run artifacts and notes.
- Outputs: signed policy-labeled envelopes.
- Failure mode: mark degraded, do not block core workflow unless configured.

2. Eval Service
- Inputs: telemetry/outcomes.
- Outputs: scorecards + optional gate result.
- Failure mode: fail-open/fail-closed policy by workflow class.

3. Recommendation Service
- Inputs: historical scorecards and route outcomes.
- Outputs: report-only recommendations.
- Failure mode: use static/default routing.

## 4. API Surface (Planned)
1. `GET /api/collaboration/peers`
2. `GET /api/collaboration/events`
3. `GET /api/evals/scorecards?workflow=...`
4. `GET /api/evals/regressions`
5. `GET /api/routing/recommendations`

## 4.1 M3.1 Implemented Interfaces (Current)
1. `agent007-p2p::CollaborationEnvelope` signed envelopes with deterministic `new_signed_at(...)`.
2. `agent007-p2p::P2pService::ingest_envelope(...)` verifies author identity signature and payload hash.
3. `agent007-p2p::CollaborationConfig` runtime gate:
- `AGENT007_COLLAB_SYNC_ENABLED`
- `AGENT007_COLLAB_VERIFY_PAYLOAD_HASH`
4. `agent007-sharing::SharingPolicy::filter_artifact(...)` applies allow/deny and redaction.
5. `agent007-sharing::SharingDecision` returns allow/block with redaction metadata.

## 5. Security + Governance
1. Signed envelope verification and peer allowlists.
2. Artifact-level policy labels and redaction rules.
3. Default deny for raw prompt/output sharing.
4. Structured audit events in run-store.

## 6. Test Strategy
1. Unit: signing/verification, policy filtering, score calculations.
2. Integration: peer sync, gate enforcement, shadow recommendation capture.
3. Regression: disabled-feature behavior parity.
4. Security: replay/tamper/unauthorized-peer scenarios.
