# Milestones & Features: agent007 V2

## Milestone Table
| Milestone | Goal | Key Features | Exit Criteria |
|---|---|---|---|
| M1 Foundation | Establish measurement baseline | Run scorecards, KPI dashboard, regression harness | Scorecards on every run, KPI panels live, regression gate active |
| M2 Reliability | Reduce avoidable failures and cost spikes | Recovery state machine, budget governor, guardrails, smart escalation | Deterministic recovery flow, budget enforcement, clear guardrail blocking |
| M3 Intelligence | Improve route/policy quality | Adaptive routing, policy learning, A/B, tool reliability, workflow optimizer | Explainable routing, reversible policy updates, experiment winner reports |
| M4 Collaboration | Enable local team sharing | p2p scaffold, mDNS, signed metadata gossip, request-response pull, trust controls | Peer discovery works, signatures verify, artifact pull succeeds |

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

## Dependency Narrative
- M1 is mandatory baseline.
- M2 depends on M1 telemetry.
- M3 depends on M1+M2 signal quality and stability.
- M4 scaffold can start earlier but production rollout should follow M2 hardening.

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
