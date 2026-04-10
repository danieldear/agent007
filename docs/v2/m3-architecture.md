# Architecture — V2 M3 Collaboration + Adaptive Foundation

## 1. Overview
M3 extends agent007 with a collaboration plane and an evaluation/control plane while preserving existing workflow execution as the primary control path.

```text
+-----------------------+        +----------------------------+
| CLI / MCP / Dashboard |------->| Workflow Engine            |
| (entrypoints)         |        | (runner + hosted)          |
+-----------------------+        +------------+---------------+
                                              |
                     +------------------------+------------------------+
                     |                                                 |
                     v                                                 v
        +---------------------------+                      +---------------------------+
        | Collaboration Plane       |                      | Eval + Policy Plane       |
        | (crates/p2p, sharing)     |                      | (learning/workflows/core) |
        +-------------+-------------+                      +-------------+-------------+
                      |                                                  |
          +-----------+-----------+                         +------------+------------+
          | Peer Sync + Identity  |                         | Scorecards + Gates      |
          | signed envelopes      |                         | routing recommendations |
          +-----------+-----------+                         +------------+------------+
                      |                                                  |
                      v                                                  v
           +------------------------+                         +-------------------------+
           | Shared Memory Artifacts|                         | Telemetry + APIs        |
           | notes/learnings only   |                         | web metrics + run_store |
           +------------------------+                         +-------------------------+
```

## 2. Module Responsibilities
1. `crates/p2p`: peer identity, discovery, pubsub, envelope verification.
2. `crates/sharing`: artifact selection policy, redaction, conflict handling.
3. `crates/core`: run-store schemas for collaboration/eval/recommendation events.
4. `crates/workflows`: regression gate hooks, shadow recommendation integration, deterministic fallback.
5. `crates/learning`: scorecard computation, baselines, recommendation confidence.
6. `crates/web`: APIs and dashboard panels for collaboration/eval/recommendation visibility.

## 3. Core Contracts
1. Collaboration envelope: `envelope_id`, `peer_id`, `artifact_type`, `payload_hash`, `timestamp`, `signature`, `policy_labels`.
2. Eval scorecard: `run_id`, `workflow`, `quality_score`, `safety_score`, `latency_ms`, `cost_usd`, `retry_count`, `regression_delta`, `gate_decision`.
3. Routing recommendation: `step_id`, `current_route`, `recommended_route`, `estimated_gain`, `confidence`, `mode`.

## 4. Data Flow
1. Workflow emits events.
2. Eval pipeline computes scorecard and gate decision.
3. Artifact policy filter classifies/redacts shareable outputs.
4. Collaboration service signs and publishes envelopes.
5. Peers verify and ingest allowed artifacts.
6. Learning service produces shadow recommendations.
