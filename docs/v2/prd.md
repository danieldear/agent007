# PRD: agent007 V2 Intelligent Orchestration + Team Collaboration

## 1. Executive Summary
V2 upgrades agent007 from static orchestration to measurable, adaptive, and collaborative orchestration. The release adds run-level evals, reliability controls, adaptive policy layers, and phased libp2p team collaboration while preserving current user workflows and compatibility across editors.

## 2. Objectives
1. Raise first-pass task/workflow success.
2. Reduce cost per successful run.
3. Reduce manual intervention frequency.
4. Enable safe incremental team knowledge sharing.

## 3. Personas
1. Solo developer using agent007 in local projects.
2. Team lead standardizing workflows/skills.
3. Platform engineer managing reliability/cost guardrails.

## 4. User Stories and Acceptance Criteria

### US-1 Run Evals ✅ Complete (M1)
As a developer, I need run-level quality metrics so I can trust improvements.
- AC-1: Every run records a normalized score with sub-signals.
- AC-2: Dashboard displays success/cost/latency/retry trends.
- AC-3: Regression suite can fail CI when quality drops beyond threshold.

### US-2 Recovery Loops ✅ Complete (M2)
As a developer, I want failed runs to auto-recover before hard failure.
- AC-1: Execution follows bounded recovery chain.
- AC-2: Retry attempts and transitions are visible in run logs.
- AC-3: Terminal fail occurs after policy-defined limits.

### US-3 Budget Governor ✅ Complete (M2)
As a team lead, I need hard cost controls.
- AC-1: Per-run and per-session budget caps configurable.
- AC-2: System downgrades strategy/model gracefully before aborting.
- AC-3: Budget events are auditable in run artifacts.

### US-4 Adaptive Routing v1 ✅ Complete (M3.3 — shadow mode)
As a user, I want model selection to adapt by task context.
- AC-1: Router uses heuristic policy inputs (task type, historical success, budget pressure).
- AC-2: Fallback route always available.
- AC-3: Routing decisions are explainable in logs.

### US-5 Smart Human Escalation ✅ Complete (M2)
As a user, I want escalation only when confidence is low.
- AC-1: Escalation triggers based on policy thresholds.
- AC-2: Approval prompts include concise rationale.
- AC-3: Users can override and continue.

### US-6 Policy Learning 📋 Planned (M3.4+)
As a platform owner, I want policy improvement over time without unstable behavior.
- AC-1: Learning store tracks policy candidates and confidence.
- AC-2: High-impact changes require approval.
- AC-3: Rollback to previous policy is one-step.

### US-7 Workflow Optimizer + A/B 📋 Planned (M3.4+)
As a team lead, I want data-driven workflow tuning.
- AC-1: A/B experiment framework can split traffic between two policies.
- AC-2: Winner criteria configurable (success/cost/latency weighted).
- AC-3: Experiment report saved in run artifacts.

### US-8 Tool Reliability Scoring 📋 Planned (M3.4+)
As a user, I want flaky tools automatically deprioritized.
- AC-1: Tool health score computed from failures/timeouts.
- AC-2: Low-score tools auto-demoted unless explicitly pinned.
- AC-3: Tool score history shown in dashboard.

### US-9 Guardrails Engine ✅ Complete (M2)
As a platform engineer, I need policy checks before risky execution.
- AC-1: Guardrails evaluated before destructive operations.
- AC-2: Violations create explicit blocked events.
- AC-3: Guardrails are configurable per project/org.

### US-10 Org Analytics Dashboard ✅ Complete (M1)
As a lead, I need cross-run trend visibility.
- AC-1: Aggregated KPIs visible by workflow/skill/model.
- AC-2: Time window filters supported.
- AC-3: Export snapshot to JSON.

### US-11 Auto-Healing Sessions ✅ Complete (M2)
As a user, I want resilience to process restarts.
- AC-1: Stale interrupted runs can be recovered to succeeded when host tokens/results arrive.
- AC-2: Recovery reason is explicit in metadata.
- AC-3: No false-positive “recorded” state when mutation is skipped.

### US-12 Libp2p Collaboration (Phase 1 local) ✅ Core Complete (M3.1) / 📋 Full mesh planned (M4)
As a team, we want lightweight local collaboration.
- AC-1: Peers auto-discover on local network (mDNS). ✅
- AC-2: Signed metadata announcements for shared artifacts. ✅ (signed envelopes + policy redaction implemented)
- AC-3: Content pulled on demand (request-response), not broadcast by default. 📋 Planned (M4)

## 5. Functional Requirements
1. Run scorecard API and storage schema.
2. Recovery state-machine integration in workflow runner.
3. Budget governor policy and enforcement hooks.
4. Adaptive routing policy API + router integration.
5. Escalation policy thresholds + approval integration.
6. Policy-learning storage/versioning + guarded apply flow.
7. A/B experiment runner and result artifacts.
8. Tool reliability tracking and ranking API.
9. Guardrails policy checks in execution path.
10. Dashboard KPI and analytics panels.
11. Restart recovery and stale-run mutation logic.
12. Libp2p mDNS peer discovery and signed event channel.

## 6. Non-Functional Requirements
1. Backward compatibility with existing commands/workflows.
2. Feature flags for all V2 capabilities.
3. Deterministic fallback path if dynamic routing fails.
4. Auditability for all automated decisions.
5. Privacy-first collaboration defaults.

## 7. Out of Scope (initial V2)
1. Default-on internet-wide p2p mesh.
2. Full autonomous policy mutation without safeguards.
3. Heavy centralized cloud dependency for collaboration.

## 8. Dependencies
1. Existing run_store + web metrics surfaces.
2. Existing learning crate components.
3. Existing workflow runner retry/evaluate hooks.
4. New crate/module for p2p collaboration.

## 9. Success Metrics
1. +15-25% first-pass success rate improvement.
2. -20% tokens per successful run.
3. -30% manual escalation rate.
4. >50% recoverable failures auto-resolved.
5. p2p local sync event propagation median <5s.

## 10. Rollout Strategy
1. Phase-gated, feature-flagged rollout.
2. Shadow mode for routing/eval before enforce mode.
3. A/B compare against baseline before broad enablement.
4. Stable GA only after KPI thresholds are sustained.
