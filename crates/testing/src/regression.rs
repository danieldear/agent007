use agent007_core::RunScorecard;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionThresholds {
    pub min_success_rate: f64,
    pub max_avg_cost_usd: f64,
    pub max_avg_latency_ms: f64,
    pub max_avg_retries: f64,
}

impl Default for RegressionThresholds {
    fn default() -> Self {
        Self {
            min_success_rate: 0.70,
            max_avg_cost_usd: 5.0,
            max_avg_latency_ms: 120_000.0,
            max_avg_retries: 2.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ScorecardSummary {
    pub sample_size: usize,
    pub completed_runs: u32,
    pub successful_runs: u32,
    pub success_rate: f64,
    pub avg_cost_usd: f64,
    pub avg_latency_ms: f64,
    pub avg_retries: f64,
    pub total_retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegressionEvaluation {
    pub passed: bool,
    pub violations: Vec<String>,
}

pub fn summarize_scorecards(scorecards: &[RunScorecard]) -> ScorecardSummary {
    let sample_size = scorecards.len();
    if sample_size == 0 {
        return ScorecardSummary::default();
    }

    let mut summary = ScorecardSummary {
        sample_size,
        ..ScorecardSummary::default()
    };

    let mut total_cost = 0.0f64;
    let mut total_latency = 0.0f64;
    let mut latency_count = 0u32;
    let mut total_retries = 0u32;

    for scorecard in scorecards {
        total_cost += scorecard.estimated_usd;
        total_retries = total_retries.saturating_add(scorecard.retry_count);
        if let Some(duration_ms) = scorecard.duration_ms {
            total_latency += duration_ms as f64;
            latency_count += 1;
        }
        if scorecard.completed {
            summary.completed_runs += 1;
            if scorecard.success {
                summary.successful_runs += 1;
            }
        }
    }

    summary.total_retries = total_retries;
    summary.avg_cost_usd = total_cost / sample_size as f64;
    summary.avg_retries = total_retries as f64 / sample_size as f64;
    if latency_count > 0 {
        summary.avg_latency_ms = total_latency / latency_count as f64;
    }
    if summary.completed_runs > 0 {
        summary.success_rate = summary.successful_runs as f64 / summary.completed_runs as f64;
    }

    summary
}

pub fn evaluate_kpi_regression(
    summary: ScorecardSummary,
    thresholds: RegressionThresholds,
) -> RegressionEvaluation {
    let mut violations = Vec::new();

    if summary.sample_size == 0 {
        violations.push("No scorecards available for regression evaluation.".to_string());
    }
    if summary.success_rate < thresholds.min_success_rate {
        violations.push(format!(
            "success_rate {:.4} is below threshold {:.4}",
            summary.success_rate, thresholds.min_success_rate
        ));
    }
    if summary.avg_cost_usd > thresholds.max_avg_cost_usd {
        violations.push(format!(
            "avg_cost_usd {:.6} exceeds threshold {:.6}",
            summary.avg_cost_usd, thresholds.max_avg_cost_usd
        ));
    }
    if summary.avg_latency_ms > thresholds.max_avg_latency_ms {
        violations.push(format!(
            "avg_latency_ms {:.2} exceeds threshold {:.2}",
            summary.avg_latency_ms, thresholds.max_avg_latency_ms
        ));
    }
    if summary.avg_retries > thresholds.max_avg_retries {
        violations.push(format!(
            "avg_retries {:.4} exceeds threshold {:.4}",
            summary.avg_retries, thresholds.max_avg_retries
        ));
    }

    RegressionEvaluation {
        passed: violations.is_empty(),
        violations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::RunStatus;
    use chrono::Utc;

    fn sample_scorecard(success: bool, retries: u32, cost: f64, latency_ms: i64) -> RunScorecard {
        let now = Utc::now();
        RunScorecard {
            schema_version: 1,
            run_id: "run".to_string(),
            kind: "workflow".to_string(),
            mode: "hosted-mcp".to_string(),
            provider: Some("mock".to_string()),
            status: if success {
                RunStatus::Succeeded
            } else {
                RunStatus::Failed
            },
            completed: true,
            success,
            started_at: now,
            finished_at: Some(now),
            duration_ms: Some(latency_ms),
            tokens: 1000,
            requests: 1,
            estimated_usd: cost,
            retry_count: retries,
            tool_calls: 1,
            tool_errors: if success { 0 } else { 1 },
            quality_score: if success { 90.0 } else { 10.0 },
            updated_at: now,
        }
    }

    #[test]
    fn summarize_scorecards_computes_expected_values() {
        let scorecards = vec![
            sample_scorecard(true, 1, 0.2, 1000),
            sample_scorecard(false, 3, 0.8, 3000),
        ];
        let summary = summarize_scorecards(&scorecards);
        assert_eq!(summary.sample_size, 2);
        assert_eq!(summary.completed_runs, 2);
        assert_eq!(summary.successful_runs, 1);
        assert!((summary.success_rate - 0.5).abs() < 0.0001);
        assert!((summary.avg_cost_usd - 0.5).abs() < 0.0001);
        assert!((summary.avg_retries - 2.0).abs() < 0.0001);
        assert!((summary.avg_latency_ms - 2000.0).abs() < 0.0001);
    }

    #[test]
    fn evaluate_kpi_regression_flags_threshold_violations() {
        let summary = ScorecardSummary {
            sample_size: 2,
            completed_runs: 2,
            successful_runs: 1,
            success_rate: 0.5,
            avg_cost_usd: 0.9,
            avg_latency_ms: 200_000.0,
            avg_retries: 3.0,
            total_retries: 6,
        };
        let thresholds = RegressionThresholds {
            min_success_rate: 0.8,
            max_avg_cost_usd: 0.4,
            max_avg_latency_ms: 120_000.0,
            max_avg_retries: 2.0,
        };
        let evaluation = evaluate_kpi_regression(summary, thresholds);
        assert!(!evaluation.passed);
        assert_eq!(evaluation.violations.len(), 4);
    }

    #[test]
    fn evaluate_kpi_regression_passes_when_within_thresholds() {
        let summary = ScorecardSummary {
            sample_size: 3,
            completed_runs: 3,
            successful_runs: 3,
            success_rate: 1.0,
            avg_cost_usd: 0.2,
            avg_latency_ms: 5000.0,
            avg_retries: 0.3,
            total_retries: 1,
        };
        let evaluation = evaluate_kpi_regression(summary, RegressionThresholds::default());
        assert!(evaluation.passed);
        assert!(evaluation.violations.is_empty());
    }
}
