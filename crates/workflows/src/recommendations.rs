use std::collections::HashMap;

use agent007_core::{RunScorecard, RunStore};
use serde::{Deserialize, Serialize};

use crate::state::WorkflowRunState;

const ROUTING_HISTORY_LIMIT: usize = 20;
const MIN_ROUTE_SAMPLES: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingRecommendation {
    pub step_id: String,
    pub current_route: String,
    pub recommended_route: String,
    pub confidence: f64,
    pub fallback_used: bool,
    pub reason: String,
    pub sample_size: usize,
}

#[derive(Debug, Default, Clone)]
struct RouteAggregate {
    samples: usize,
    quality_sum: f64,
    success_count: usize,
}

impl RouteAggregate {
    fn record(&mut self, scorecard: &RunScorecard) {
        self.samples += 1;
        self.quality_sum += scorecard.quality_score;
        if scorecard.success {
            self.success_count += 1;
        }
    }

    fn avg_quality(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.quality_sum / self.samples as f64
        }
    }

    fn success_rate(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.success_count as f64 / self.samples as f64
        }
    }
}

pub fn recommend_route_for_step(
    store: &RunStore,
    workflow: &str,
    step_id: &str,
    current_route: &str,
    candidate_routes: &[String],
    exclude_run_id: Option<&str>,
) -> RoutingRecommendation {
    if candidate_routes.len() < 2 {
        return fallback_recommendation(
            step_id,
            current_route,
            "router has fewer than two candidate routes",
            0,
        );
    }

    let aggregates =
        match gather_route_history(store, workflow, step_id, candidate_routes, exclude_run_id) {
            Ok(aggregates) => aggregates,
            Err(error) => {
                return fallback_recommendation(
                    step_id,
                    current_route,
                    format!("route history unavailable: {error}"),
                    0,
                );
            }
        };

    let total_samples: usize = aggregates.values().map(|entry| entry.samples).sum();
    if total_samples < MIN_ROUTE_SAMPLES || aggregates.len() < 2 {
        return fallback_recommendation(
            step_id,
            current_route,
            format!("insufficient historical route data ({total_samples} sample(s))"),
            total_samples,
        );
    }

    let Some((best_route, best_stats)) = aggregates
        .iter()
        .max_by(|left, right| compare_route_aggregates(left.1, right.1))
    else {
        return fallback_recommendation(step_id, current_route, "no historical route data", 0);
    };

    if best_route == current_route {
        return RoutingRecommendation {
            step_id: step_id.to_string(),
            current_route: current_route.to_string(),
            recommended_route: current_route.to_string(),
            confidence: normalize_confidence(best_stats.samples as f64 / total_samples as f64),
            fallback_used: true,
            reason: format!(
                "current route already matches the best-known historical outcome ({:.1} avg quality across {} sample(s))",
                best_stats.avg_quality(),
                best_stats.samples,
            ),
            sample_size: total_samples,
        };
    }

    let current_stats = aggregates.get(current_route);
    let current_quality = current_stats
        .map(RouteAggregate::avg_quality)
        .unwrap_or(0.0);
    let current_samples = current_stats.map(|stats| stats.samples).unwrap_or(0);
    let quality_gap = (best_stats.avg_quality() - current_quality).max(0.0);
    let evidence_factor =
        ((best_stats.samples + current_samples) as f64 / MIN_ROUTE_SAMPLES as f64).min(1.0);
    let confidence = normalize_confidence((quality_gap / 100.0) * evidence_factor);

    RoutingRecommendation {
        step_id: step_id.to_string(),
        current_route: current_route.to_string(),
        recommended_route: best_route.clone(),
        confidence,
        fallback_used: false,
        reason: format!(
            "{} has stronger historical outcomes than {} ({:.1} vs {:.1} avg quality; success {:.0}% vs {:.0}%)",
            best_route,
            current_route,
            best_stats.avg_quality(),
            current_quality,
            best_stats.success_rate() * 100.0,
            current_stats.map(RouteAggregate::success_rate).unwrap_or(0.0) * 100.0,
        ),
        sample_size: total_samples,
    }
}

fn gather_route_history(
    store: &RunStore,
    workflow: &str,
    step_id: &str,
    candidate_routes: &[String],
    exclude_run_id: Option<&str>,
) -> Result<HashMap<String, RouteAggregate>, agent007_core::CoreError> {
    let scorecards =
        store.recent_scorecards_for_workflow(workflow, exclude_run_id, ROUTING_HISTORY_LIMIT)?;
    let mut aggregates = HashMap::new();

    for scorecard in scorecards {
        let Some(state) = store.read_json_artifact_optional::<WorkflowRunState>(
            &scorecard.run_id,
            "workflow-state.json",
        )?
        else {
            continue;
        };
        let Some(route) = state
            .steps
            .iter()
            .find(|step| step.id == step_id)
            .and_then(|step| step.selected_route.clone())
        else {
            continue;
        };
        if !candidate_routes.iter().any(|candidate| candidate == &route) {
            continue;
        }
        aggregates
            .entry(route)
            .or_insert_with(RouteAggregate::default)
            .record(&scorecard);
    }

    Ok(aggregates)
}

fn fallback_recommendation(
    step_id: &str,
    current_route: &str,
    reason: impl Into<String>,
    sample_size: usize,
) -> RoutingRecommendation {
    RoutingRecommendation {
        step_id: step_id.to_string(),
        current_route: current_route.to_string(),
        recommended_route: current_route.to_string(),
        confidence: 0.0,
        fallback_used: true,
        reason: reason.into(),
        sample_size,
    }
}

fn compare_route_aggregates(left: &RouteAggregate, right: &RouteAggregate) -> std::cmp::Ordering {
    left.avg_quality()
        .partial_cmp(&right.avg_quality())
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            left.success_rate()
                .partial_cmp(&right.success_rate())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| left.samples.cmp(&right.samples))
}

fn normalize_confidence(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::{RunStatus, RunStore};
    use chrono::Utc;

    fn write_route_history(
        store: &RunStore,
        workflow: &str,
        step_id: &str,
        selected_route: &str,
        quality_score: f64,
    ) {
        let run = store
            .create_run("workflow-test", "baseline", "standalone", Some("mock"))
            .unwrap();
        store
            .write_json_artifact(
                &run.id,
                "workflow-request.json",
                &serde_json::json!({
                    "workflow": workflow,
                    "task": "baseline"
                }),
            )
            .unwrap();
        store
            .write_json_artifact(
                &run.id,
                "workflow-state.json",
                &serde_json::json!({
                    "workflow": workflow,
                    "task": "baseline",
                    "status": "succeeded",
                    "steps_total": 1,
                    "steps_completed": 1,
                    "completed_steps": [step_id],
                    "skipped_steps": [],
                    "retry_counts": {},
                    "recovery_retry_counts": {},
                    "outputs": {},
                    "budget_used": { "tokens": 0, "estimated_usd": 0.0 },
                    "degradation_count": 0,
                    "reliability_transitions": [],
                    "reliability_events": [],
                    "eval_gate_decision": null,
                    "routing_recommendations": [],
                    "steps": [{
                        "id": step_id,
                        "agent": "Router",
                        "status": "completed",
                        "attempts": 1,
                        "output_key": "classification",
                        "output_preview": selected_route,
                        "selected_route": selected_route,
                        "selected_target": null,
                        "error": null
                    }],
                    "pending_approval": null,
                    "approval_decisions": {},
                    "last_error": null
                }),
            )
            .unwrap();
        let finished = store.finish_run(&run.id, true, "baseline ok").unwrap();
        let retry_count = ((100.0 - quality_score).max(0.0) / 4.0).round() as u32;
        store
            .write_json_artifact(
                &run.id,
                "run-scorecard.json",
                &RunScorecard {
                    schema_version: 1,
                    run_id: run.id.clone(),
                    kind: "workflow-test".to_string(),
                    workflow: Some(workflow.to_string()),
                    mode: finished.mode.clone(),
                    provider: finished.provider.clone(),
                    status: RunStatus::Succeeded,
                    completed: true,
                    success: true,
                    started_at: finished.started_at,
                    finished_at: finished.finished_at,
                    duration_ms: Some(100),
                    tokens: 0,
                    requests: 1,
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    estimated_usd: 0.0,
                    cost_mode: agent007_core::RunCostMode::FallbackEstimate,
                    retry_count,
                    tool_calls: 0,
                    tool_errors: 0,
                    quality_score,
                    updated_at: Utc::now(),
                },
            )
            .unwrap();
    }

    #[test]
    fn recommendation_falls_back_without_enough_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        write_route_history(&store, "router-flow", "classify", "api-work", 60.0);

        let recommendation = recommend_route_for_step(
            &store,
            "router-flow",
            "classify",
            "ui-work",
            &["ui-work".to_string(), "api-work".to_string()],
            None,
        );

        assert_eq!(recommendation.recommended_route, "ui-work");
        assert!(recommendation.fallback_used);
        assert_eq!(recommendation.sample_size, 1);
    }

    #[test]
    fn recommendation_prefers_higher_quality_route() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        write_route_history(&store, "router-flow", "classify", "ui-work", 42.0);
        write_route_history(&store, "router-flow", "classify", "ui-work", 44.0);
        write_route_history(&store, "router-flow", "classify", "api-work", 95.0);
        write_route_history(&store, "router-flow", "classify", "api-work", 92.0);

        let recommendation = recommend_route_for_step(
            &store,
            "router-flow",
            "classify",
            "ui-work",
            &["ui-work".to_string(), "api-work".to_string()],
            None,
        );

        assert_eq!(recommendation.recommended_route, "api-work");
        assert!(!recommendation.fallback_used);
        assert!(recommendation.confidence > 0.4);
        assert_eq!(recommendation.sample_size, 4);
    }
}
