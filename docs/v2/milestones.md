# Milestones & Features: agent007 V2

## Milestone Table
| Milestone | Status | Goal | Key Features | Exit Criteria |
|---|---|---|---|---|
| M1 Foundation | ✅ Complete | Establish measurement baseline | Run scorecards, KPI dashboard, regression harness | Scorecards on every run, KPI panels live, regression gate active |
| M2 Reliability | ✅ Complete | Reduce avoidable failures and cost spikes | Recovery state machine, budget governor, guardrails, smart escalation | Deterministic recovery flow, budget enforcement, clear guardrail blocking |
| M3 Intelligence | ✅ Complete (M3.1–M3.3) / 🔄 In Progress (M3.4) | Improve route/policy quality + collaboration core | Eval gates, adaptive shadow, signed envelopes, policy redaction, workflow hardening | Explainable routing, reversible policy updates, experiment winner reports |
| M4 Collaboration | 📋 Planned | Enable local team sharing (libp2p mesh) | p2p scaffold, mDNS, signed metadata gossip, request-response pull, trust controls | Peer discovery works, signatures verify, artifact pull succeeds |
| M5 Shared Workspace | 📋 Planned | Shared memory, analysis artifacts, and task delegation across peers | shared/ memory scope, shareable entry tags, artifact subscriptions, cross-peer task delegation with approval | Team memory queryable via RAG, artifact feed live in dashboard, task delegation approved and audited |

## Feature Breakdown by Milestone
### M1
1. Run scorecard schema + writer.
2. Dashboard KPI views.
3. Regression suite + thresholds.

### M2
1. Recovery transition engine.
2. Budget cap + degradation policy.
3. Guardrails execution hook.
4. Confidence-based escalation.
5. Feature-flagged rollout controls, optional workflow-level reliability overrides, and transition/event audit trail (`docs/v2/m2-reliability.md`).

### M3
1. Adaptive routing adapter.
2. Policy-learning candidate lifecycle.
3. A/B experimentation.
4. Tool reliability scoring + demotion.
5. Workflow optimizer recommendation.

### M4
1. `crates/p2p` + identity.
2. mDNS discovery.
3. Signed gossipsub metadata announcements.
4. Request-response artifact pull.
5. Peer trust/allowlist controls.

### M5
1. `shared/` scope in `memory::MemoryStore`.
2. `shareable` entry metadata flag + pre-share dry-run command.
3. Policy redaction extended to memory entries.
4. Pull API: peer A requests entries from peer B by key or tag.
5. Artifact subscription API + dashboard "Team Artifacts" view.
6. Cross-peer task delegation with approval gate.
7. Task audit trail in both peers' run stores.
8. Revocation: tombstone envelopes for withdrawn entries and cancelled tasks.

## Dependency Narrative
- M1 is mandatory baseline.
- M2 depends on M1 telemetry.
- M3 depends on M1+M2 signal quality and stability.
- M4 scaffold can start earlier but production rollout should follow M2 hardening.
- M5 Phase 1 (shared memory pull) depends on M4 request-response. M5 Phase 3 (task delegation) requires M4 trust controls to be hardened first.

## Recommended Execution Order
1. M1 -> 2. M2 -> 3. M3 -> 4. M4.

## Parallel Workstreams
1. Core Runtime (workflow/policy).
2. Learning & Analytics.
3. Routing & Experiments.
4. Collaboration/P2P.

## Project Definition of Done
1. All milestone exit criteria met.
2. Feature flags for major V2 capabilities.
3. Regression suite shows no unacceptable degradation.
4. Documentation and rollout controls published.
5. End-to-end demo includes adaptive run, recovery, analytics, and local p2p sharing.

## M3 Planning Docs
1. `docs/v2/m3-prd.md`
2. `docs/v2/m3-architecture.md`
3. `docs/v2/m3-design.md`
4. `docs/v2/m3-milestones.md`
5. `docs/v2/m3-collaboration-evals.md`

## M5 Planning Docs
1. `docs/v2/m5-shared-workspace.md`
