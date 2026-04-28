# Milestones and Feature Delivery

## Recommended Order
1. M1 Core Runtime Reliability
2. M2 Visibility and Productization
3. M3 Controlled Rollout and Quality Gate

## Milestone Table
| Milestone | Goal | Key Features | Dependencies | Exit Criteria |
|---|---|---|---|---|
| M1 | Consistent retrieval + execution behavior | Warmup indexing bounds, shared skill executor path, telemetry artifact generation, persona policy enforcement | none | core paths green + artifacts persisted |
| M2 | User-facing observability | run-detail API extension, dashboard telemetry/policy/token cards, docs updates | M1 | artifacts visible in UI and validated |
| M3 | Safe rollout | strict-mode rollout matrix, KPI baseline tracking, rollback playbook | M2 | measured rollout decision gates |

## Parallel Workstreams
1. Backend: retriever/executor/policy and artifact persistence.
2. Frontend: API consumption and dashboard UI sections.
3. Docs/Ops: rollout controls, known issues, runbooks.

## Definition of Done
1. Build/test pipeline green.
2. Run-detail artifacts visible and accurate.
3. Strict mode validated in controlled test.
4. Operational toggles documented.
