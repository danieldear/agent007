# Milestones and Feature Delivery

## Recommended Order
1. M1 Core Runtime Reliability
2. M2 Visibility and Productization
3. M3 Controlled Rollout and Quality Gate
4. M4 Runtime Sessions, Agent Collaboration, and TUI Usability
5. M6 Repo-Native Structural Intelligence and Graph Retrieval

## Milestone Table
| Milestone | Status | Goal | Key Features | Dependencies | Exit Criteria |
|---|---|---|---|---|---|
| M1 | ✅ Complete | Consistent retrieval + execution behavior | Warmup indexing bounds, shared skill executor path, telemetry artifact generation, persona policy enforcement | none | core paths green + artifacts persisted |
| M2 | ✅ Complete | User-facing observability | run-detail API extension, dashboard telemetry/policy/token cards, docs updates | M1 | artifacts visible in UI and validated |
| M3 | 📋 Planned | Safe rollout | strict-mode rollout matrix, KPI baseline tracking, rollback playbook | M2 | measured rollout decision gates |
| M4 | 🚧 In Progress | Stronger long-lived runtime and operator UX | runtime session inventory, CLI status watch/filter, operator TUI v1, structured agent messages, provider readiness, artifact/mock viewer v1, repository skill catalog import with bulk install, artifact versioning, session notes, memory lifecycle improvements | M2 | sessions resumable, runtime visible in dashboard/TUI, first operator-grade terminal flow usable, visual artifacts reviewable in dashboard |
| M5 | ✅ Complete | True multi-agent orchestration | Sub-orchestrator wired to CLI (`agent agent run`) and MCP (`agent007_agent_run`); parallel JoinSet worker dispatch; dynamic replan on blockers; cross-agent memory synthesis (`last_run`); new AgentEvent variants (`WorkerResult`, `WorkerBlocked`, `TaskFailed`) flowing to dashboard metrics; docs in `docs/multi-agent.md` | M1 | `cargo test --workspace` green, agent run end-to-end functional via CLI and MCP |
| M6 | 📋 Planned | Make the repo itself a first-class intelligence source | structural repo graph, call/caller/usage/import/doc links, repo-first hybrid RAG, incremental graph refresh, graph-aware ETR queries, memory cross-linking to structural evidence | M4, M5 | repo graph persists locally, graph-aware queries work through ETR, repo corpus is default retrieval source, structural freshness visible |

## Parallel Workstreams
1. Backend: retriever/executor/policy and artifact persistence.
2. Frontend: API consumption, provider readiness cards, runtime status, and artifact preview UI.
3. Docs/Ops: rollout controls, known issues, runbooks.

## Definition of Done
1. Build/test pipeline green.
2. Run-detail artifacts visible and accurate.
3. Strict mode validated in controlled test.
4. Operational toggles documented.
