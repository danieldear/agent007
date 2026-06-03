use agent007_core::{RunScorecard, RunStatus, RunStore, TOKEN_PRICE_PER_TOKEN_USD};
use agent007_testing::{
    compare_scorecard_to_baseline, summarize_scorecards, BaselineComparison, BaselineThresholds,
    ScorecardSummary,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::WorkflowError;
use crate::types::{BudgetUsed, EvalGateMode, WorkflowDef};

#[derive(Debug, Clone)]
pub struct EvalGatePolicy {
    pub enabled: bool,
    pub release_class: bool,
    pub mode: EvalGateMode,
    pub baseline_window: usize,
    pub min_baseline_runs: usize,
    pub thresholds: BaselineThresholds,
}

impl Default for EvalGatePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            release_class: false,
            mode: EvalGateMode::FailOpen,
            baseline_window: 10,
            min_baseline_runs: 3,
            thresholds: BaselineThresholds::default(),
        }
    }
}

impl EvalGatePolicy {
    pub fn from_env() -> Self {
        Self {
            enabled: env_bool("AGENT007_EVAL_GATES_ENABLED", false),
            release_class: env_bool("AGENT007_EVAL_GATES_RELEASE_CLASS", false),
            mode: env_mode("AGENT007_EVAL_GATES_MODE").unwrap_or_default(),
            baseline_window: env_usize("AGENT007_EVAL_GATES_BASELINE_WINDOW", 10),
            min_baseline_runs: env_usize("AGENT007_EVAL_GATES_MIN_BASELINE_RUNS", 3),
            thresholds: BaselineThresholds {
                max_quality_score_drop: env_f64(
                    "AGENT007_EVAL_GATES_MAX_QUALITY_SCORE_DROP",
                    BaselineThresholds::default().max_quality_score_drop,
                ),
                max_cost_usd_increase: env_f64(
                    "AGENT007_EVAL_GATES_MAX_COST_USD_INCREASE",
                    BaselineThresholds::default().max_cost_usd_increase,
                ),
                max_latency_ms_increase: env_f64(
                    "AGENT007_EVAL_GATES_MAX_LATENCY_MS_INCREASE",
                    BaselineThresholds::default().max_latency_ms_increase,
                ),
                max_retry_increase: env_f64(
                    "AGENT007_EVAL_GATES_MAX_RETRY_INCREASE",
                    BaselineThresholds::default().max_retry_increase,
                ),
            },
        }
    }

    pub fn from_workflow(def: &WorkflowDef) -> Self {
        let mut policy = Self::from_env();
        if let Some(config) = &def.eval_gate {
            if let Some(enabled) = config.enabled {
                policy.enabled = enabled;
            }
            if let Some(release_class) = config.release_class {
                policy.release_class = release_class;
            }
            if let Some(mode) = &config.mode {
                policy.mode = mode.clone();
            }
            if let Some(window) = config.baseline_window {
                policy.baseline_window = window.max(1);
            }
            if let Some(min_runs) = config.min_baseline_runs {
                policy.min_baseline_runs = min_runs.max(1);
            }
            if let Some(thresholds) = &config.thresholds {
                if let Some(value) = thresholds.max_quality_score_drop {
                    policy.thresholds.max_quality_score_drop = value;
                }
                if let Some(value) = thresholds.max_cost_usd_increase {
                    policy.thresholds.max_cost_usd_increase = value;
                }
                if let Some(value) = thresholds.max_latency_ms_increase {
                    policy.thresholds.max_latency_ms_increase = value;
                }
                if let Some(value) = thresholds.max_retry_increase {
                    policy.thresholds.max_retry_increase = value;
                }
            }
        }
        policy
    }

    pub fn active(&self) -> bool {
        self.enabled && self.release_class
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowEvalGateDecisionKind {
    Pass,
    Warn,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowEvalGateDecision {
    pub workflow: String,
    pub mode: EvalGateMode,
    pub decision: WorkflowEvalGateDecisionKind,
    pub baseline_window: usize,
    pub min_baseline_runs: usize,
    pub baseline_sample_size: usize,
    pub thresholds: BaselineThresholds,
    pub current_scorecard: RunScorecard,
    pub baseline_summary: Option<ScorecardSummary>,
    pub comparison: Option<BaselineComparison>,
    pub reason_codes: Vec<String>,
    pub message: String,
}

pub fn evaluate_workflow_eval_gate(
    store: &RunStore,
    run_id: &str,
    workflow: &str,
    budget_used: &BudgetUsed,
    policy: &EvalGatePolicy,
) -> Result<Option<WorkflowEvalGateDecision>, WorkflowError> {
    if !policy.active() {
        return Ok(None);
    }

    let current_scorecard = candidate_scorecard(store, run_id, workflow, budget_used)?;
    let baseline_runs = store
        .recent_scorecards_for_workflow(workflow, Some(run_id), policy.baseline_window)
        .map_err(|error| WorkflowError::StepFailed {
            id: "eval-gate".to_string(),
            reason: format!("failed to load baseline scorecards: {error}"),
        })?;

    if baseline_runs.len() < policy.min_baseline_runs {
        // Bootstrap should never hard-fail a workflow. A fail-closed gate is
        // intended to block *known regressions* against a valid baseline, not
        // punish first runs or newly introduced workflows that have not yet
        // accumulated enough scorecards.
        let decision = WorkflowEvalGateDecisionKind::Warn;
        return Ok(Some(WorkflowEvalGateDecision {
            workflow: workflow.to_string(),
            mode: policy.mode.clone(),
            decision,
            baseline_window: policy.baseline_window,
            min_baseline_runs: policy.min_baseline_runs,
            baseline_sample_size: baseline_runs.len(),
            thresholds: policy.thresholds.clone(),
            current_scorecard,
            baseline_summary: None,
            comparison: None,
            reason_codes: vec!["insufficient-baseline".to_string()],
            message: format!(
                "baseline sample size {} is below required minimum {}",
                baseline_runs.len(),
                policy.min_baseline_runs
            ),
        }));
    }

    let baseline_summary = summarize_scorecards(&baseline_runs);
    let comparison = compare_scorecard_to_baseline(
        &current_scorecard,
        baseline_summary.clone(),
        policy.thresholds.clone(),
    );
    let decision = if comparison.passed {
        WorkflowEvalGateDecisionKind::Pass
    } else {
        match policy.mode {
            EvalGateMode::FailOpen => WorkflowEvalGateDecisionKind::Warn,
            EvalGateMode::FailClosed => WorkflowEvalGateDecisionKind::Block,
        }
    };
    let message = if comparison.passed {
        format!(
            "eval gate passed against {} baseline runs",
            baseline_summary.sample_size
        )
    } else {
        format!(
            "eval gate detected regression against {} baseline runs",
            baseline_summary.sample_size
        )
    };

    Ok(Some(WorkflowEvalGateDecision {
        workflow: workflow.to_string(),
        mode: policy.mode.clone(),
        decision,
        baseline_window: policy.baseline_window,
        min_baseline_runs: policy.min_baseline_runs,
        baseline_sample_size: baseline_summary.sample_size,
        thresholds: policy.thresholds.clone(),
        current_scorecard,
        baseline_summary: Some(baseline_summary),
        reason_codes: comparison.violation_codes.clone(),
        comparison: Some(comparison),
        message,
    }))
}

pub fn persist_eval_gate_artifacts(
    store: &RunStore,
    run_id: &str,
    decision: &WorkflowEvalGateDecision,
) -> Result<(), WorkflowError> {
    store
        .write_json_artifact(run_id, "eval-gate-decision.json", decision)
        .map_err(|error| WorkflowError::StepFailed {
            id: "eval-gate".to_string(),
            reason: format!("failed to persist eval gate artifact: {error}"),
        })?;
    store
        .append_note(
            run_id,
            "workflow-eval-gate-decision",
            serde_json::to_value(decision).unwrap_or_else(|_| serde_json::json!({})),
        )
        .map_err(|error| WorkflowError::StepFailed {
            id: "eval-gate".to_string(),
            reason: format!("failed to append eval gate event: {error}"),
        })?;
    Ok(())
}

fn candidate_scorecard(
    store: &RunStore,
    run_id: &str,
    workflow: &str,
    budget_used: &BudgetUsed,
) -> Result<RunScorecard, WorkflowError> {
    let detail = store
        .load_run(run_id)
        .map_err(|error| WorkflowError::StepFailed {
            id: "eval-gate".to_string(),
            reason: format!("failed to load current run metadata: {error}"),
        })?;
    let existing =
        store
            .read_run_scorecard_optional(run_id)
            .map_err(|error| WorkflowError::StepFailed {
                id: "eval-gate".to_string(),
                reason: format!("failed to read current run scorecard: {error}"),
            })?;

    let finished_at = Some(Utc::now());
    let duration_ms = finished_at.map(|finished| {
        finished
            .signed_duration_since(detail.metadata.started_at)
            .num_milliseconds()
    });

    let mut scorecard = existing.unwrap_or(RunScorecard {
        schema_version: 1,
        run_id: detail.metadata.id.clone(),
        kind: detail.metadata.kind.clone(),
        workflow: Some(workflow.to_string()),
        mode: detail.metadata.mode.clone(),
        provider: detail.metadata.provider.clone(),
        status: RunStatus::Succeeded,
        completed: true,
        success: true,
        started_at: detail.metadata.started_at,
        finished_at,
        duration_ms,
        tokens: budget_used.tokens,
        requests: 0,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        estimated_usd: budget_used.tokens as f64 * TOKEN_PRICE_PER_TOKEN_USD,
        cost_mode: agent007_core::RunCostMode::FallbackEstimate,
        retry_count: 0,
        tool_calls: 0,
        tool_errors: 0,
        quality_score: 0.0,
        updated_at: Utc::now(),
    });

    scorecard.workflow = Some(workflow.to_string());
    scorecard.status = RunStatus::Succeeded;
    scorecard.completed = true;
    scorecard.success = true;
    scorecard.finished_at = finished_at;
    scorecard.duration_ms = duration_ms;
    scorecard.tokens = budget_used.tokens;
    scorecard.estimated_usd = if budget_used.estimated_usd > 0.0 {
        budget_used.estimated_usd
    } else {
        budget_used.tokens as f64 * TOKEN_PRICE_PER_TOKEN_USD
    };
    scorecard.updated_at = Utc::now();
    scorecard.quality_score = computed_quality_score(&scorecard);

    Ok(scorecard)
}

fn computed_quality_score(scorecard: &RunScorecard) -> f64 {
    if !scorecard.completed {
        return 0.0;
    }

    let mut score = if scorecard.success { 100.0 } else { 0.0 };
    score -= scorecard.retry_count as f64 * 4.0;
    score -= scorecard.tool_errors as f64 * 6.0;
    score -= (scorecard.tokens as f64 / 100_000.0).min(15.0);
    if let Some(duration_ms) = scorecard.duration_ms {
        score -= (duration_ms as f64 / 60_000.0).min(20.0);
    }
    score.clamp(0.0, 100.0)
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .unwrap_or(default)
}

fn env_mode(key: &str) -> Option<EvalGateMode> {
    let value = std::env::var(key).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "fail-open" | "fail_open" | "open" => Some(EvalGateMode::FailOpen),
        "fail-closed" | "fail_closed" | "closed" => Some(EvalGateMode::FailClosed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn sample_scorecard(run_id: &str, quality: f64, cost: f64, retries: u32) -> RunScorecard {
        let now = Utc::now();
        RunScorecard {
            schema_version: 1,
            run_id: run_id.to_string(),
            kind: "workflow".to_string(),
            workflow: Some("release".to_string()),
            mode: "standalone".to_string(),
            provider: Some("mock".to_string()),
            status: RunStatus::Succeeded,
            completed: true,
            success: true,
            started_at: now,
            finished_at: Some(now),
            duration_ms: Some(1000),
            tokens: 1000,
            requests: 1,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            estimated_usd: cost,
            cost_mode: agent007_core::RunCostMode::FallbackEstimate,
            retry_count: retries,
            tool_calls: 1,
            tool_errors: 0,
            quality_score: quality,
            updated_at: now,
        }
    }

    #[test]
    fn policy_active_requires_enabled_release_class() {
        let policy = EvalGatePolicy {
            enabled: true,
            release_class: true,
            ..EvalGatePolicy::default()
        };
        assert!(policy.active());
    }

    #[test]
    fn compare_scorecard_to_baseline_warns_on_regression_in_fail_open() {
        let current = sample_scorecard("run-1", 60.0, 0.8, 3);
        let baseline = summarize_scorecards(&[
            sample_scorecard("run-2", 92.0, 0.2, 0),
            sample_scorecard("run-3", 91.0, 0.2, 0),
            sample_scorecard("run-4", 93.0, 0.2, 0),
        ]);
        let comparison =
            compare_scorecard_to_baseline(&current, baseline, BaselineThresholds::default());
        assert!(!comparison.passed);
        assert!(comparison
            .violation_codes
            .contains(&"cost-increase".to_string()));
    }

    #[test]
    fn insufficient_baseline_warns_even_when_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("workflow-test", "ship release", "hosted-mcp", Some("mock"))
            .unwrap();
        let policy = EvalGatePolicy {
            enabled: true,
            release_class: true,
            mode: EvalGateMode::FailClosed,
            min_baseline_runs: 3,
            ..EvalGatePolicy::default()
        };

        let decision = evaluate_workflow_eval_gate(
            &store,
            &run.id,
            "brand-new-workflow",
            &BudgetUsed::default(),
            &policy,
        )
        .unwrap()
        .expect("decision expected");

        assert_eq!(decision.decision, WorkflowEvalGateDecisionKind::Warn);
        assert_eq!(decision.baseline_sample_size, 0);
        assert!(decision
            .reason_codes
            .contains(&"insufficient-baseline".to_string()));
    }
}
