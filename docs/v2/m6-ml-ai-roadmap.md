# Roadmap — V2 M6 ML/AI Optimization

## 1. Why M6 Exists
M1-M3 established observability, reliability controls, and adaptive shadow recommendations.  
M6 turns those signals into learned policies that can outperform direct, one-shot LLM usage in measurable ways:

1. Higher first-pass success rate.
2. Lower token and cost per successful run.
3. Lower retry and tool-error rates.
4. Better retrieval relevance with less prompt noise.

## 2. Current Baseline (Already in Repo)
1. Memory + retrieval stack with LanceDB + fallback retrieval:
   - `crates/memory/src/vectordb/lancedb.rs`
   - `crates/memory/src/retriever.rs`
   - `crates/memory/src/store.rs`
2. Learning loop primitives:
   - reward scoring: `crates/learning/src/scorer.rs`
   - prompt optimization: `crates/learning/src/optimizer.rs`
   - procedural insights: `crates/learning/src/insight.rs`
3. Workflow quality + routing foundations:
   - route recommendation: `crates/workflows/src/recommendations.rs`
   - eval gates: `crates/workflows/src/eval_gates.rs`
   - reliability controls: `crates/workflows/src/reliability.rs`
4. Run telemetry + scorecards:
   - `crates/core/src/run_store.rs`
   - `crates/web/src/metrics.rs`

## 3. Gap to Close
Current adaptation is mostly heuristic and threshold-driven.  
M6 introduces supervised/weakly-supervised decision layers with staged rollout.

## 4. Target Architecture (High Level)
```text
[Run Artifacts + Workflow State + Retrieval Telemetry + Scorecards]
                             |
                             v
                    [Feature Builder Layer]
                             |
              +--------------+--------------+
              |                             |
              v                             v
       [Offline Evaluation]          [Model Training]
      (replay/counterfactual)   (route/rank/risk scorers)
              |                             |
              +--------------+--------------+
                             v
                     [Policy Decision API]
            (route/model/tool/recovery recommendation)
                             |
                             v
                  [Workflow Runner + Skills]
                             |
                             v
                    [Telemetry + Feedback]
```

## 5. Phased Delivery Plan

### M6.1 Instrumentation + Offline Eval (2 weeks)
1. Build structured feature dataset from run artifacts and workflow state.
2. Add retrieval quality telemetry artifacts per run/step.
3. Add offline replay harness for route/rank/risk comparisons vs heuristic baseline.
4. Define promotion thresholds and rollback criteria.

Exit criteria:
1. Offline dataset generated reproducibly.
2. Baseline heuristic metrics captured.
3. Candidate models can be scored without production impact.

### M6.2 Learned Decisions in Shadow Mode (2-4 weeks)
1. Learned route scorer emits recommendations only (no traffic control).
2. Retrieval ranker produces shadow ordering and quality deltas.
3. Failure-risk model predicts high-risk steps and expected recovery benefit.

Exit criteria:
1. Shadow recommendations logged with confidence + reason codes.
2. No regression to current execution behavior.
3. Sufficient sample size for promotion decision.

### M6.3 Controlled Activation (2-4 weeks)
1. Canary rollout for selected workflows/projects.
2. Automatic fallback to heuristic policy on confidence/health threshold breach.
3. Dashboard view for policy lift vs baseline.

Exit criteria:
1. Measurable KPI lift sustained for promotion window.
2. Rollback proven in simulation and live canary.
3. Audit trail complete for all automated decisions.

## 6. Core Candidate Models (Start Lightweight)
1. Route selection: logistic regression or GBDT with contextual features.
2. Retrieval ranking: lightweight relevance scorer using retrieval + outcome weak labels.
3. Failure risk: binary classifier for step failure/tool error probability.

Note: start with simple, interpretable models before heavier alternatives.

## 7. KPI Contract for Go/No-Go
1. `success_rate` improves without increased safety incidents.
2. `avg_cost_usd` decreases for equal or better quality.
3. `avg_retries_per_run` and `tool_error_rate` decrease.
4. Retrieval fallback ratio decreases.
5. `quality_score` improves or remains stable with lower token usage.

## 8. Token Efficiency via Local Deterministic Tasks
M6 includes a non-ML but high-impact token-efficiency track:

1. Move repetitive deterministic operations to local tools/scripts:
   - git hygiene
   - build/test wrappers
   - log parsing
   - setup/bootstrap scripts
   - device/adb helper scripts
2. Call tools first, call LLM second:
   - structured local output becomes compact context for the model.
3. Prefer cached artifacts over re-deriving steps in prompts.
4. Use context compaction aggressively for command output.

Expected impact:
1. Lower prompt length on repetitive tasks.
2. Lower model latency/cost.
3. More predictable execution behavior.

## 9. Risks and Mitigations
1. Risk: Noisy labels from run outcomes.
   - Mitigation: confidence-weighted training data and strict evaluation windows.
2. Risk: Overfitting to one project style.
   - Mitigation: cross-project validation and fallback to heuristics.
3. Risk: Unsafe automatic policy changes.
   - Mitigation: shadow mode, canary gates, one-step rollback, approval requirement for high-impact changes.

## 10. Dependencies
1. M1 scorecards and telemetry completeness.
2. M2 reliability controls.
3. M3.4 hardening for safe rollout.
4. Stable run artifact schemas.

## 11. Execution Notes
1. Start M6.1 immediately; no behavior change required.
2. Keep all learned paths feature-flagged.
3. Treat M6 as iterative optimization, not a single big-bang release.
