# Architecture: agent007 V2

## 1. Architecture Overview
V2 introduces an explicit control loop around existing execution paths.

```text
User/Host Request
   -> Dispatch Layer (CLI/MCP)
   -> Orchestration Layer (skills/workflows)
   -> Execution Layer (models/tools)
   -> Signal Layer (events/run_store/learning)
   -> Policy Layer (routing/recovery/budget/guardrails)
   -> Presentation Layer (dashboard/analytics)
   -> Collaboration Layer (p2p events + pull)
```

Design principle: extend current crates instead of replacing them.

## 2. Component Breakdown and Responsibilities
1. Routing + Policy: `crates/models/src/router.rs` with policy-aware route selection.
2. Workflow Reliability: `crates/workflows/src/runner.rs` + `state.rs` recovery transitions.
3. Learning/Evaluation: `crates/learning/{collector,scorer,store,optimizer}.rs`.
4. Run Artifact Core: `crates/core/src/run_store.rs`.
5. Budget/Guardrails/Escalation policy runtime: `crates/workflows/src/reliability.rs` integrated into `runner.rs` and `hosted.rs`, with workflow-level overrides via `WorkflowDef.reliability`.
6. Web Analytics: `crates/web/src/{metrics,api,dashboard}.rs`.
7. Collaboration Transport: new `crates/p2p`.

## 3. Interface Contracts
- `RouteDecision { provider, model, confidence, rationale_code, fallback }`
- `RecoveryDecision { action, reason, attempt, max_attempts }`
- `RunScorecard { score, completion, tool_error_rate, retries, latency_ms, tokens, cost_usd }`
- `GuardrailResult { allowed, reason, escalate }`
- `P2PAnnouncement { kind, artifact_id, hash, author_peer, ts, signature }`

## 4. Data Flow
1. Request enters orchestrator via CLI/MCP.
2. Router chooses route with policy context.
3. Workflow runner executes with guardrails.
4. Recovery engine decides retries/rewinds/escalations (evaluator and execution-recovery retries tracked separately).
5. Events/artifacts persist in run_store.
6. Learning collector updates score/policy signals.
7. Dashboard surfaces trends.
8. Optional p2p layer broadcasts signed metadata and supports pull.

## 5. Technology Choices and Trade-offs
1. Incremental existing crates: fast and safer, but needs strong boundaries.
2. mDNS-first libp2p: zero-config local value, remote deferred.
3. Metadata announce + pull content: privacy and bandwidth efficient.
4. Feature-flagged rollout: safer but more test permutations.

## 6. NFR Targets
1. Reliability: deterministic terminal states, no retry storms.
2. Security: signed p2p metadata, allowlists, guardrails.
3. Observability: explicit events for auto decisions.
4. Performance: bounded policy overhead.
5. Scalability: responsive analytics on growing run history.

## 7. Module-to-Feature Mapping
1. Run evals -> learning scorer/collector + web metrics.
2. Recovery loops -> workflows runner/state.
3. Budget governor -> core policy budget.
4. Adaptive routing -> models router + policy adapter.
5. Human escalation -> core policy escalation + approvals.
6. Policy learning -> learning store/optimizer.
7. A/B experiments -> new learning experiments module.
8. Tool reliability -> learning signals + dashboard.
9. Guardrails engine -> core policy guardrails + execution hooks.
10. Org analytics -> web metrics + API query layer.
11. Auto-healing sessions -> run_store + hosted serve recovery path.
12. Libp2p collaboration -> crates/p2p.

## 8. Implementation Sequencing
- Wave 1: scorecards + dashboard + regression harness.
- Wave 2: recovery + budget + guardrails.
- Wave 3: adaptive routing + learning + A/B.
- Wave 4: local-first p2p collaboration.
