# Milestones — V2 M3 Collaboration + Adaptive Foundation

| Milestone | Status | Duration | Goal | Exit Criteria |
|---|---|---:|---|---|
| M3.1 Collaboration Core | ✅ Complete | 2 weeks | Secure optional peer sync for approved artifacts | Envelope signing + policy filter + local mesh tests pass |
| M3.2 Eval Gates | ✅ Complete | 2 weeks | Introduce scorecards and regression blocking | Scorecards emitted + gate policy enforced + dashboard visible |
| M3.3 Adaptive Shadow | ✅ Complete | 2 weeks | Emit recommendations without behavior risk | Recommendation confidence surfaced + fallback validated |
| M3.4 Hardening & Rollout | 🔄 In Progress | 1-2 weeks | Production readiness and controlled enablement | Security/perf suites pass + rollout matrix complete |

## Feature Breakdown

### M3.1
1. Peer identity/signing envelopes (`crates/p2p`, `crates/sharing`).
2. Artifact policy engine and redaction filters (`crates/sharing`).
3. Collaboration event persistence (`crates/core`).

### M3.2
1. Score model + baseline comparator (`crates/learning`).
2. Regression gate hooks (`crates/workflows`).
3. Eval APIs/dashboard (`crates/web`, frontend).

### M3.3
1. Recommendation engine (`crates/learning`).
2. Shadow capture hooks (`crates/workflows`).
3. Recommendation APIs/UI (`crates/web`, frontend).

### M3.4
1. Security hardening (tamper/replay/allowlist tests).
2. Load/perf/soak validation.
3. Rollout controls and runbook publication.

Current backend progress:
1. Tamper rejection is covered in `crates/p2p/tests/local_sync.rs`.
2. Replay rejection is enforced in `agent007-p2p::P2pService::ingest_envelope(...)`.
3. Allowlist coverage currently maps to registered peers only; unknown peers are rejected during ingest.

## Parallel Workstreams
1. Workstream A: collaboration infra (`p2p`, `sharing`, `core`).
2. Workstream B: eval + recommendation (`learning`, `workflows`).
3. Workstream C: API/dashboard/docs (`web`, frontend, docs).

## Definition of Done
1. M3 acceptance criteria satisfied.
2. Backward compatibility validated with features disabled.
3. Hosted and standalone parity verified.
4. Security and performance gates green.
5. Documentation published under `docs/v2`.
