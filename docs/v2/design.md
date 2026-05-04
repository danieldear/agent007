# Design Document: agent007 V2

## Summary
This design unifies PRD goals with architecture decisions for phased delivery.

## Product Requirements Summary
- Outcomes: higher success, lower cost, lower manual intervention, collaboration.
- Strategy: feature-flagged staged activation.
- Scope: evals, recovery, budget, routing, learning, A/B, guardrails, analytics, auto-healing, libp2p.

## System Design
### Control Loop
1. Execute task/workflow.
2. Capture events/metrics.
3. Compute score + reliability signals.
4. Update policy candidates.
5. Apply guarded policy updates.
6. Surface results in dashboard.

### Collaboration Loop
1. Discover peers (mDNS).
2. Publish signed metadata event.
3. Pull artifacts on demand.
4. Validate and persist.

## Component Specs
1. Routing adapter in models/router.
2. Reliability transitions in workflows runner/hosted with workflow-level overrides (`WorkflowDef.reliability`).
3. Score engine in learning scorer.
4. Budget governor in workflows reliability runtime.
5. Guardrails pre-checks in workflows reliability runtime using rendered-context + normalized matching.
6. Tool health metrics in learning + web.
7. Collaboration transport in crates/p2p.

## Data Models
- Policy profile (weights/thresholds/mode).
- Experiment config (control/treatment split).
- Tool health snapshot.
- Peer identity/trust metadata.

## Quality, Security, Performance
- Quality: regression suite gate.
- Security: signed metadata, allowlists, strict defaults.
- Performance: bounded policy compute and retries.

## Delivery Waves
1. Wave 1 Foundation.
2. Wave 2 Reliability.
3. Wave 3 Intelligence.
4. Wave 4 Collaboration.

## Decision Rationale
Prioritizes practical reliability and measurement before deeper adaptive automation.

## Implementation Addendum (2026-05-03)
Current implementation has also delivered operational foundations used by V2 tracks:
1. Extension adapters and install APIs.
2. MCP registry + RAG source management endpoints.
3. Tool registry import/search/test/approval model.
4. Always-on learning worker execution in both `run` and `serve`.
5. Memory stats API and dashboard surfacing for confidence/type distribution visibility.
