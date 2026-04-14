use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path as FsPath, PathBuf};
use ts_rs::TS;

use agent007_core::paths::{
    agent007_global_home, agent007_home, agent007_project_home, agent007_write_home,
};
use agent007_sharing;
use agent007_testing::{evaluate_kpi_regression, summarize_scorecards, RegressionThresholds};
use agent007_workflows::{
    WorkflowError, WorkflowLoader, WorkflowRunRequest, WorkflowRunState, WorkflowSourceRef,
};

use crate::server::AppState;

const RESUME_TARGET_ARTIFACT: &str = "resume-target.json";
const RESUME_SOURCE_ARTIFACT: &str = "resume-source.json";
const EXTERNAL_WORKFLOW_CONTROL_ERROR: &str =
    "This workflow is controlled by the client that started it. Review, approve, and continue it there; the web dashboard is read-only for external workflow runs.";

fn dashboard_controls_workflow(kind: &str) -> bool {
    kind.starts_with("workflow-web-")
}

// ── request/response shapes ───────────────────────────────────────────────────

#[derive(Deserialize, Serialize, TS)]
#[ts(export, export_to = "frontend/src/types/")]
pub struct RunRequest {
    pub task: String,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "frontend/src/types/")]
pub struct RunResponse {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session: Option<String>,
}

#[derive(Deserialize, Serialize, TS)]
#[ts(export, export_to = "frontend/src/types/")]
pub struct ApprovalRequest {
    #[ts(optional)]
    pub step: Option<String>,
    pub decision: String,
    #[ts(optional)]
    pub content: Option<String>,
}

#[derive(Deserialize, Serialize, TS)]
#[ts(export, export_to = "frontend/src/types/")]
pub struct SkillRunRequest {
    pub trigger: String,
    #[serde(default)]
    pub args: String,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "frontend/src/types/")]
pub struct SkillRunResponse {
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResumeTargetRef {
    session: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResumeSourceRef {
    source_session: String,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "frontend/src/types/")]
pub struct StatusResponse {
    #[ts(type = "unknown[]")]
    pub agents: Vec<Value>,
    #[ts(type = "unknown[]")]
    pub tasks: Vec<Value>,
    pub avg_reward: f64,
}

#[derive(Debug, Default, Deserialize)]
pub struct ScorecardsQuery {
    #[serde(default = "default_scorecards_limit")]
    pub limit: usize,
}

#[derive(Debug, Default, Deserialize)]
pub struct RegressionEvaluateQuery {
    #[serde(default = "default_scorecards_limit")]
    pub limit: usize,
    pub min_success_rate: Option<f64>,
    pub max_avg_cost_usd: Option<f64>,
    pub max_avg_latency_ms: Option<f64>,
    pub max_avg_retries: Option<f64>,
}

fn default_scorecards_limit() -> usize {
    100
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// `POST /api/run` — submit a task to the orchestrator.
pub async fn run_handler(
    State(state): State<AppState>,
    Json(payload): Json<RunRequest>,
) -> impl IntoResponse {
    if !state.standalone_mode {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "This dashboard is running in hosted MCP mode. Run tasks from Codex, Claude, or Cursor via MCP, or configure Ollama / a standalone provider for direct dashboard execution."
            })),
        )
            .into_response();
    }

    let cancel = state.cancel.clone();
    let prompt_store = Arc::new(std::sync::Mutex::new(
        agent007_core::types::PromptStore::default(),
    ));
    let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
    let provider = state.model_router.route("task");
    let run = match store.create_run(
        "web-run",
        &payload.task,
        "standalone",
        Some(provider.name()),
    ) {
        Ok(run) => run,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let trace = match store
        .spawn_dispatcher_trace(
            run.id.clone(),
            state.dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        )
        .await
    {
        Ok(handle) => Some(handle),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let orchestrator = Arc::new(agent007_core::orchestrator::OrchestratorAgent::new(
        state.dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        state.model_router.clone(),
        prompt_store,
        cancel,
        4,
    ));

    let core_task = agent007_core::Task::new(&payload.task);
    match orchestrator.run(core_task).await {
        Ok(result) => {
            if let Some(trace) = trace {
                let _ = trace.await;
            }
            let _ = store.write_text_artifact(&run.id, "output.txt", &result.output);
            let _ = store.finish_run(&run.id, true, &result.output);
            (
                StatusCode::OK,
                Json(RunResponse {
                    message: result.output,
                    session: Some(run.id),
                }),
            )
                .into_response()
        }
        Err(e) => {
            if let Some(trace) = trace {
                trace.abort();
            }
            let _ = store.finish_run(&run.id, false, e.to_string());
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

/// `GET /api/skills` — list skills from project-local and global `.agent007/skills/`.
/// Each skill includes a `source` field: `"project"` or `"global"`.
pub async fn skills_handler(State(_state): State<AppState>) -> impl IntoResponse {
    let project_dir = agent007_project_home().map(|p| p.join("skills"));
    let global_dir = agent007_global_home().join("skills");

    // Pre-compute triggers available in the global dir so we can detect built-in skills
    // that were seeded into the project dir during `init` — those should show as "global".
    let global_triggers: std::collections::HashSet<String> = load_skills_from_dir(&global_dir)
        .into_iter()
        .map(|skill| skill.trigger().to_string())
        .collect();

    let mut skills: Vec<Value> = Vec::new();
    let mut seen_triggers: std::collections::HashSet<String> = std::collections::HashSet::new();

    let dirs: Vec<(std::path::PathBuf, &str)> = {
        let mut v: Vec<(std::path::PathBuf, &str)> = Vec::new();
        if let Some(p) = project_dir {
            v.push((p, "project"));
        }
        v.push((global_dir, "global"));
        v
    };

    for (dir, source) in &dirs {
        for skill in load_skills_from_dir(dir) {
            let trigger = skill.trigger().to_string();
            if seen_triggers.insert(trigger.clone()) {
                let effective_source = if *source == "project" && global_triggers.contains(&trigger)
                {
                    "global"
                } else {
                    source
                };
                skills.push(skill_json(&skill, effective_source));
            }
        }
    }

    Json(Value::Array(skills)).into_response()
}

/// `DELETE /api/skills/:trigger` — delete a skill file by its trigger slug.
pub async fn skill_delete_handler(Path(trigger): Path<String>) -> impl IntoResponse {
    let target_trigger = format!("/{}", trigger.trim_start_matches('/'));

    // Search dirs: project-local first, then global.
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(p) = agent007_project_home() {
        dirs.push(p.join("skills"));
    }
    dirs.push(agent007_global_home().join("skills"));

    // Scan frontmatter to find the file with matching trigger.
    // This handles the case where the filename doesn't match the trigger slug
    // (e.g. senior-ml-engineer.md with trigger: /skill).
    for dir in &dirs {
        for skill in load_skills_from_dir(dir) {
            if skill.trigger() == target_trigger {
                return match remove_skill_entry(&skill) {
                    Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
                    Err(e) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": e.to_string() })),
                    )
                        .into_response(),
                };
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "skill not found" })),
    )
        .into_response()
}

/// `POST /api/skills/run` — run a skill by trigger.
pub async fn skills_run_handler(
    State(state): State<AppState>,
    Json(payload): Json<SkillRunRequest>,
) -> impl IntoResponse {
    if !state.standalone_mode {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "This dashboard is running in hosted MCP mode. Run skills from Codex, Claude, or Cursor via MCP, or configure Ollama / a standalone provider for direct dashboard execution."
            })),
        )
            .into_response();
    }

    // Build a minimal SkillExecutor (dry-run, no real VectorDB).
    let embedder = Arc::new(agent007_models::MockProvider::with_embedding_dim(
        "",
        "mock-embed",
        384,
    )) as Arc<dyn agent007_models::EmbeddingProvider>;

    let db = Arc::new(NoOpVectorDB) as Arc<dyn agent007_memory::VectorDB>;
    let retriever = Arc::new(agent007_memory::Retriever::new(embedder, db, 5));

    let memory_store = memory_store_for_web();
    let memory = memory_store.global();
    let global_store = Arc::new(agent007_memory::store::MemoryStore::new(
        agent007_global_home().join("memory"),
    ));
    let global_memory = global_store.scoped("global");

    let model = state.model_router.clone() as Arc<dyn agent007_models::ModelProvider>;

    let executor = agent007_skills::SkillExecutor::new(model, retriever, memory)
        .with_global_memory(global_memory);
    let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
    let run = match store.create_run(
        "web-skill-run",
        &format!("skill:{} {}", payload.trigger, payload.args),
        "standalone",
        None,
    ) {
        Ok(run) => run,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let skills_dir = agent007_home().join("skills");
    let loader = agent007_skills::SkillLoader::new(&skills_dir);

    let skills = match loader.load_all() {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let skill =
        match skills.into_iter().find(|s| s.trigger() == payload.trigger) {
            Some(s) => s,
            None => return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::json!({ "error": format!("skill not found: {}", payload.trigger) }),
                ),
            )
                .into_response(),
        };

    match executor.execute(&skill, &payload.args).await {
        Ok(output) => {
            let _ = store.finish_run(
                &run.id,
                true,
                format!("skill '{}' completed", payload.trigger),
            );
            (
                StatusCode::OK,
                Json(SkillRunResponse {
                    output,
                    session: Some(run.id),
                }),
            )
                .into_response()
        }
        Err(e) => {
            let _ = store.finish_run(&run.id, false, e.to_string());
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

/// `GET /api/status` — return current agent + task snapshot.
///
/// In this implementation the dispatcher does not expose an introspection API,
/// so we return empty lists with a placeholder avg_reward. A follow-on plan
/// can add `Dispatcher::snapshot()` to expose live data.
pub async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let m = crate::metrics::snapshot_with_shared_state(
        state.metrics.lock().await.clone(),
        agent007_home(),
    );
    let tasks = m
        .recent_tasks
        .iter()
        .map(|task| {
            serde_json::json!({
                "id": task.id,
                "task": task.task,
                "status": task.status,
                "agent": task.agent,
                "tokens": task.tokens,
                "started_at": task.started_at,
                "finished_at": task.finished_at,
            })
        })
        .collect();
    let agents = if m.active_agents == 0 {
        vec![]
    } else {
        vec![serde_json::json!({
            "name": m.model_provider,
            "status": if state.runtime_mode == "hosted-mcp" {
                "hosted"
            } else if state.runtime_mode == "dry-run" {
                "dry-run"
            } else {
                "active"
            },
            "count": m.active_agents,
        })]
    };
    Json(StatusResponse {
        agents,
        tasks,
        avg_reward: m.avg_reward,
    })
}

/// `GET /api/stats` — comprehensive dashboard metrics.
pub async fn stats_handler(State(state): State<AppState>) -> impl IntoResponse {
    let mut m = crate::metrics::snapshot_with_shared_state(
        state.metrics.lock().await.clone(),
        agent007_home(),
    );

    // Collect all home dirs (project-local first, then global) — deduplicated
    let mut homes = Vec::new();
    if let Some(proj) = agent007_project_home() {
        homes.push(proj);
    }
    let global = agent007_global_home();
    if !homes.contains(&global) {
        homes.push(global);
    }

    let skills_count: u32 = homes
        .iter()
        .map(|h| count_dir_files(&h.join("skills"), "md"))
        .sum();
    let workflows_count: u32 = homes
        .iter()
        .map(|h| {
            count_dir_files(&h.join("workflows"), "yaml")
                + count_dir_files(&h.join("workflows"), "yml")
        })
        .sum();
    let personas_count: u32 = homes
        .iter()
        .map(|h| count_dir_files(&h.join("personas"), "toml"))
        .sum();
    // Memory is recursive (user/, project/ subdirs) — count from write home only to avoid double-counting
    let memory_keys = count_dir_files(&agent007_write_home().join("memory"), "md");
    m.update_inventory(skills_count, workflows_count, personas_count, memory_keys);

    let mut snapshot = serde_json::to_value(m).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = snapshot.as_object_mut() {
        obj.insert(
            "project_name".to_string(),
            serde_json::json!(state.project_name),
        );
        obj.insert(
            "project_path".to_string(),
            serde_json::json!(state.project_path),
        );
        // Ensure runtime_mode is always present (metrics struct has it, but guarantee it)
        if !obj.contains_key("runtime_mode") {
            obj.insert(
                "runtime_mode".to_string(),
                serde_json::json!(state.runtime_mode),
            );
        }
    }
    Json(snapshot).into_response()
}

/// `GET /api/scorecards` — recent run scorecards (newest first).
pub async fn scorecards_handler(Query(query): Query<ScorecardsQuery>) -> impl IntoResponse {
    let limit = query.limit.clamp(1, 500);
    let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
    match store.list_runs(limit) {
        Ok(runs) => {
            let mut scorecards = Vec::new();
            for run in runs {
                if let Ok(scorecard) = store.ensure_run_scorecard_artifact(&run.id) {
                    scorecards.push(scorecard);
                }
            }
            Json(serde_json::to_value(scorecards).unwrap_or_else(|_| serde_json::json!([])))
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/regression/evaluate` — compare current KPI snapshot against thresholds.
pub async fn regression_evaluate_handler(
    Query(query): Query<RegressionEvaluateQuery>,
) -> impl IntoResponse {
    let limit = query.limit.clamp(1, 500);
    let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
    let runs = match store.list_runs(limit) {
        Ok(runs) => runs,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let mut scorecards = Vec::new();
    for run in runs {
        if let Ok(scorecard) = store.ensure_run_scorecard_artifact(&run.id) {
            scorecards.push(scorecard);
        }
    }

    let summary = summarize_scorecards(&scorecards);
    let mut thresholds = RegressionThresholds::default();
    if let Some(value) = query.min_success_rate {
        thresholds.min_success_rate = value;
    }
    if let Some(value) = query.max_avg_cost_usd {
        thresholds.max_avg_cost_usd = value;
    }
    if let Some(value) = query.max_avg_latency_ms {
        thresholds.max_avg_latency_ms = value;
    }
    if let Some(value) = query.max_avg_retries {
        thresholds.max_avg_retries = value;
    }

    let evaluation = evaluate_kpi_regression(summary.clone(), thresholds.clone());
    Json(serde_json::json!({
        "window": limit,
        "sample_size": scorecards.len(),
        "summary": summary,
        "thresholds": thresholds,
        "passed": evaluation.passed,
        "violations": evaluation.violations,
    }))
    .into_response()
}

pub async fn runs_handler(State(_state): State<AppState>) -> impl IntoResponse {
    let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
    match store.list_runs(25) {
        Ok(runs) => Json(serde_json::to_value(runs).unwrap_or_else(|_| serde_json::json!([])))
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn run_detail_handler(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
    match store.load_run(&id) {
        Ok(run) => {
            let output_text = store
                .read_text_artifact_optional(&id, "output.txt")
                .ok()
                .flatten();
            let workflow_request = store
                .read_json_artifact_optional::<serde_json::Value>(&id, "workflow-request.json")
                .ok()
                .flatten();
            let workflow_source = store
                .read_json_artifact_optional::<serde_json::Value>(&id, "workflow-source.json")
                .ok()
                .flatten();
            let workflow_state = store
                .read_json_artifact_optional::<serde_json::Value>(&id, "workflow-state.json")
                .ok()
                .flatten();
            Json(serde_json::json!({
                "run": run,
                "output_text": output_text,
                "workflow_request": workflow_request,
                "workflow_source": workflow_source,
                "workflow_state": workflow_state,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn run_approval_handler(
    State(_state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<ApprovalRequest>,
) -> impl IntoResponse {
    let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
    let detail = match store.load_run(&id) {
        Ok(detail) => detail,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    if !dashboard_controls_workflow(&detail.metadata.kind) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": EXTERNAL_WORKFLOW_CONTROL_ERROR })),
        )
            .into_response();
    }
    let mut state: agent007_workflows::WorkflowRunState =
        match store.read_json_artifact(&id, "workflow-state.json") {
            Ok(state) => state,
            Err(e) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        };

    let step_id = payload.step.clone().or_else(|| {
        state
            .pending_approval
            .as_ref()
            .map(|pending| pending.step_id.clone())
    });
    let Some(step_id) = step_id else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "no pending approval found for this run" })),
        )
            .into_response();
    };

    let decision = match parse_approval_decision(&payload.decision, payload.content.clone()) {
        Ok(decision) => decision,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };

    state.record_approval_decision(&step_id, decision);
    match store.write_json_artifact(&id, "workflow-state.json", &state) {
        Ok(()) => Json(serde_json::json!({
            "ok": true,
            "session": id,
            "step": step_id,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn run_resume_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if !state.standalone_mode {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "This dashboard is running in hosted MCP mode. Configure Ollama or a standalone provider to resume workflows directly from the web UI."
            })),
        )
            .into_response();
    }

    let Some(workflow_runner) = state.workflow_runner.clone() else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Workflow runtime is not available in this web server instance."
            })),
        )
            .into_response();
    };

    let store = Arc::new(agent007_core::RunStore::new(
        agent007_home().join("sessions"),
    ));
    let detail = match store.load_run(&id) {
        Ok(detail) => detail,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    if !dashboard_controls_workflow(&detail.metadata.kind) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": EXTERNAL_WORKFLOW_CONTROL_ERROR })),
        )
            .into_response();
    }

    if detail.metadata.status == agent007_core::run_store::RunStatus::Succeeded {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "completed runs cannot be resumed; use replay instead"
            })),
        )
            .into_response();
    }

    let request: WorkflowRunRequest = match store.read_json_artifact(&id, "workflow-request.json") {
        Ok(request) => request,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    if let Ok(Some(existing)) =
        store.read_json_artifact_optional::<ResumeTargetRef>(&id, RESUME_TARGET_ARTIFACT)
    {
        let status = store
            .load_run(&existing.session)
            .map(|run| run.metadata.status)
            .unwrap_or(agent007_core::run_store::RunStatus::Running);
        return Json(serde_json::json!({
            "ok": true,
            "status": match status {
                agent007_core::run_store::RunStatus::Running => "running",
                agent007_core::run_store::RunStatus::AwaitingApproval => "awaiting-approval",
                agent007_core::run_store::RunStatus::Succeeded => "succeeded",
                agent007_core::run_store::RunStatus::Failed => "failed",
            },
            "session": existing.session,
            "workflow": request.workflow,
            "already_resumed": true,
        }))
        .into_response();
    }
    let workflow_ref =
        match store.read_json_artifact_optional::<WorkflowSourceRef>(&id, "workflow-source.json") {
            Ok(Some(source)) => source.workflow_ref,
            Ok(None) => request.workflow.clone(),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        };

    let workflow_state: WorkflowRunState =
        match store.read_json_artifact(&id, "workflow-state.json") {
            Ok(state) => state,
            Err(e) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        };

    if workflow_state.pending_approval.is_some() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "approval is still pending for this workflow run"
            })),
        )
            .into_response();
    }

    let def = match load_workflow_from_dashboard_dirs(&workflow_ref) {
        Ok(def) => def,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let resumed = match store.create_run(
        "workflow-web-resume",
        &format!("{}: {}", request.workflow, request.task),
        "standalone",
        None,
    ) {
        Ok(run) => run,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    if let Err(e) = store.write_json_artifact(
        &resumed.id,
        "workflow-source.json",
        &WorkflowSourceRef { workflow_ref },
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    if let Err(e) = store.write_json_artifact(
        &resumed.id,
        RESUME_SOURCE_ARTIFACT,
        &ResumeSourceRef {
            source_session: id.clone(),
        },
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    if let Err(e) = store.write_json_artifact(
        &id,
        RESUME_TARGET_ARTIFACT,
        &ResumeTargetRef {
            session: resumed.id.clone(),
        },
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let runner = workflow_runner.resume_from(store.clone(), resumed.id.clone(), workflow_state);
    match runner.run(&def, &request.task).await {
        Ok(_) => {
            let _ = store.finish_run(
                &resumed.id,
                true,
                format!("workflow '{}' completed", request.workflow),
            );
            Json(serde_json::json!({
                "ok": true,
                "status": "succeeded",
                "session": resumed.id,
                "workflow": request.workflow,
            }))
            .into_response()
        }
        Err(WorkflowError::ApprovalRequired { id: step_id }) => {
            let _ = store.finish_run_with_status(
                &resumed.id,
                agent007_core::run_store::RunStatus::AwaitingApproval,
                format!("approval required for step '{}'", step_id),
            );
            Json(serde_json::json!({
                "ok": true,
                "status": "awaiting-approval",
                "session": resumed.id,
                "workflow": request.workflow,
                "step": step_id,
            }))
            .into_response()
        }
        Err(e) => {
            let _ = store.finish_run(&resumed.id, false, e.to_string());
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

fn count_dir_files(dir: &std::path::Path, ext: &str) -> u32 {
    count_dir_files_recursive(dir, ext)
}

fn count_dir_files_recursive(dir: &std::path::Path, ext: &str) -> u32 {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut count = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += count_dir_files_recursive(&path, ext);
        } else if path.extension().and_then(|x| x.to_str()) == Some(ext) {
            count += 1;
        }
    }
    count
}

fn parse_approval_decision(
    decision: &str,
    content: Option<String>,
) -> Result<agent007_workflows::approval::ApprovalDecision, String> {
    use agent007_workflows::approval::{ApprovalDecision, ApprovalDecisionKind};

    let kind = match decision.trim().to_lowercase().as_str() {
        "approve" | "approved" | "yes" | "y" => ApprovalDecisionKind::Approve,
        "deny" | "denied" | "no" | "n" => ApprovalDecisionKind::Deny,
        "edit" | "edited" => ApprovalDecisionKind::Edit,
        other => {
            return Err(format!(
                "unknown approval decision '{}'; use approve, deny, or edit",
                other
            ));
        }
    };

    if kind == ApprovalDecisionKind::Edit && content.as_deref().unwrap_or("").trim().is_empty() {
        return Err("content is required when decision=edit".to_string());
    }

    Ok(ApprovalDecision {
        decision: kind,
        content,
    })
}

// ── Persona CRUD ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PersonaSaveRequest {
    pub name: String,
    pub description: String,
    pub preferred_model: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub system_prompt: Option<String>,
}

pub async fn personas_list_handler(State(_state): State<AppState>) -> impl IntoResponse {
    let personas_dir = agent007_home().join("personas");
    let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
        .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
    use agent007_core::PersonaProvider;
    let personas: Vec<Value> = registry
        .list()
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "description": p.description,
                "preferred_model": p.preferred_model,
                "allowed_tools": p.allowed_tools,
                "system_prompt": p.system_prompt,
            })
        })
        .collect();
    Json(Value::Array(personas)).into_response()
}

pub async fn persona_save_handler(
    State(_state): State<AppState>,
    Json(payload): Json<PersonaSaveRequest>,
) -> impl IntoResponse {
    let personas_dir = agent007_write_home().join("personas");
    if let Err(e) = std::fs::create_dir_all(&personas_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let filename = payload
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase();
    let path = personas_dir.join(format!("{filename}.toml"));

    let tools = payload.allowed_tools.unwrap_or_default();
    let tools_str = tools
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let model = payload.preferred_model.as_deref().unwrap_or("codex");
    let prompt = payload.system_prompt.as_deref().unwrap_or("");

    let content = format!(
        "name            = \"{}\"\n\
         description     = \"{}\"\n\
         preferred_model = \"{}\"\n\
         allowed_tools   = [{}]\n\n\
         system_prompt   = \"\"\"\n{}\n\"\"\"\n",
        payload.name,
        payload.description.replace('"', "\\\""),
        model,
        tools_str,
        prompt,
    );

    match std::fs::write(&path, &content) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "path": path.display().to_string() }))
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn persona_delete_handler(
    State(_state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let personas_dir = agent007_home().join("personas");
    let filename = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase();
    let path = personas_dir.join(format!("{filename}.toml"));

    if path.exists() {
        match std::fs::remove_file(&path) {
            Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "not found" })),
        )
            .into_response()
    }
}

// ── Workflow CRUD ──────────────────────────────────────────────────────────────

/// Returns all directories to search for workflow YAML files, in priority order:
/// `AGENT007_HOME/workflows/` if explicitly set, otherwise project-local
/// `.agent007/workflows/` first, then global `~/.agent007/workflows/`.
/// Mirrors `configured_workflow_dirs()` in the MCP server so the dashboard always
/// shows the same set of workflows that the MCP tool `agent007_workflow_list` returns.
fn workflow_dirs() -> Vec<std::path::PathBuf> {
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        return vec![std::path::PathBuf::from(home).join("workflows")];
    }

    let mut dirs = Vec::new();
    if let Some(project) = agent007_project_home() {
        dirs.push(project.join("workflows"));
    }
    let global = agent007_global_home().join("workflows");
    if !dirs.iter().any(|d| d == &global) {
        dirs.push(global);
    }
    dirs
}

fn load_workflow_from_dashboard_dirs(
    name: &str,
) -> Result<agent007_workflows::WorkflowDef, String> {
    for workflows_dir in workflow_dirs() {
        let loader = WorkflowLoader::new(workflows_dir.clone());
        match loader.load_named(name) {
            Ok(def) => return Ok(def),
            Err(agent007_workflows::WorkflowError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Workflow '{}' not found or invalid in {}: {}",
                    name,
                    workflows_dir.display(),
                    error
                ));
            }
        }
    }

    let searched = workflow_dirs()
        .into_iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Workflow '{}' not found in configured workflow dirs: {}",
        name, searched
    ))
}

fn sanitize_file_stem(raw: &str, fallback: &str) -> String {
    let sanitized = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

pub async fn workflows_list_handler(State(_state): State<AppState>) -> impl IntoResponse {
    let global = agent007_global_home().join("workflows");

    // Pre-compute workflow names present in the global dir so project-local copies
    // of built-in workflows (seeded by `init`) are shown as "global", not "project".
    let global_names: std::collections::HashSet<String> = {
        let mut s = std::collections::HashSet::new();
        if let Ok(entries) = std::fs::read_dir(&global) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str());
                if ext == Some("yaml") || ext == Some("yml") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        s.insert(stem.to_string());
                    }
                }
            }
        }
        s
    };

    let mut seen = std::collections::HashSet::new();
    let mut result: Vec<Value> = Vec::new();
    for wf_dir in workflow_dirs() {
        let is_global = wf_dir == global;
        if let Ok(entries) = std::fs::read_dir(&wf_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str());
                if ext == Some("yaml") || ext == Some("yml") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if seen.insert(stem.to_string()) {
                            let source = if is_global || global_names.contains(stem) {
                                "global"
                            } else {
                                "project"
                            };
                            result.push(serde_json::json!({
                                "name": stem,
                                "source": source,
                            }));
                        }
                    }
                }
            }
        }
    }
    result.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    Json(result).into_response()
}

pub async fn workflow_get_handler(
    State(_state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let requested = name.trim();
    let safe_name = sanitize_file_stem(requested, "");
    if safe_name.is_empty() || safe_name != requested {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid workflow name" })),
        )
            .into_response();
    }

    let path = workflow_dirs()
        .into_iter()
        .flat_map(|dir| {
            [
                dir.join(format!("{safe_name}.yaml")),
                dir.join(format!("{safe_name}.yml")),
            ]
        })
        .find(|p| p.exists());
    let Some(path) = path else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "not found" })),
        )
            .into_response();
    };

    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_yaml::from_str::<Value>(&content) {
            Ok(val) => Json(val).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Workflow validate ─────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ValidateStructural {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct ValidateLlm {
    pub available: bool,
    pub score: Option<u8>,
    pub summary: Option<String>,
    pub issues: Vec<String>,
    pub suggestions: Vec<String>,
}

#[derive(Serialize)]
pub struct WorkflowValidateResponse {
    pub valid: bool,
    pub structural: ValidateStructural,
    pub llm: ValidateLlm,
}

/// `POST /api/workflows/validate` — structurally validate a workflow and
/// optionally ask the LLM for semantic feedback (standalone mode only).
pub async fn workflow_validate_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let structural = validate_structural(&payload);
    let llm = if state.standalone_mode {
        validate_with_llm(&state, &payload).await
    } else {
        ValidateLlm {
            available: false,
            score: None,
            summary: None,
            issues: vec![],
            suggestions: vec![],
        }
    };
    let valid = structural.errors.is_empty();
    Json(WorkflowValidateResponse {
        valid,
        structural,
        llm,
    })
    .into_response()
}

fn validate_structural(workflow: &Value) -> ValidateStructural {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if workflow
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty()
    {
        warnings.push("Workflow has no name — it will be saved as 'untitled'".into());
    }

    let steps = match workflow.get("steps").and_then(|s| s.as_array()) {
        Some(s) if !s.is_empty() => s,
        _ => {
            errors.push("Workflow has no steps".into());
            return ValidateStructural { errors, warnings };
        }
    };

    // Collect all IDs and outputs in a first pass
    let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut outputs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (i, step) in steps.iter().enumerate() {
        let id = step
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            errors.push(format!("Step {i} has no 'id'"));
        } else if !ids.insert(id.clone()) {
            errors.push(format!("Duplicate step id: '{id}'"));
        }
        if let Some(out) = step.get("output").and_then(|v| v.as_str()) {
            outputs.insert(out.to_string());
        }
    }

    // Second pass: per-step validation
    for step in steps {
        let id = step.get("id").and_then(|v| v.as_str()).unwrap_or("?");

        if step
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty()
        {
            errors.push(format!("Step '{id}' has no 'agent'"));
        }

        let prompt = step.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        if prompt.is_empty() {
            warnings.push(format!("Step '{id}' has an empty prompt"));
        }

        match step
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("execute")
        {
            "evaluator" => {
                match step.get("evaluate") {
                    None => errors.push(format!(
                        "Evaluator step '{id}' is missing 'evaluate' config"
                    )),
                    Some(eval) => {
                        if eval
                            .get("decision_field")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .is_empty()
                        {
                            errors.push(format!(
                                "Evaluator step '{id}': evaluate.decision_field is required"
                            ));
                        }
                        if eval
                            .get("on_pass")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .is_empty()
                        {
                            warnings
                                .push(format!("Evaluator step '{id}': evaluate.on_pass is empty"));
                        }
                        if eval
                            .get("on_fail")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .is_empty()
                        {
                            warnings
                                .push(format!("Evaluator step '{id}': evaluate.on_fail is empty"));
                        }
                    }
                }
                if !prompt.to_lowercase().contains("json") {
                    warnings.push(format!("Evaluator step '{id}': prompt should instruct the agent to respond with JSON"));
                }
            }
            "router" => {
                let routes = step.get("routes").and_then(|r| r.as_array());
                if routes.map_or(true, |r| r.is_empty()) {
                    errors.push(format!("Router step '{id}' is missing 'routes' config"));
                } else {
                    let has_default = routes
                        .unwrap()
                        .iter()
                        .any(|r| r.get("default").and_then(|v| v.as_bool()).unwrap_or(false));
                    if !has_default {
                        warnings.push(format!("Router step '{id}' has no default route — unmatched classifications will stall"));
                    }
                }
            }
            _ => {}
        }

        // depends_on reference check
        if let Some(deps) = step.get("depends_on").and_then(|d| d.as_array()) {
            for dep in deps {
                let dep_id = dep.as_str().unwrap_or("");
                if dep_id == id {
                    errors.push(format!("Step '{id}' has a self-dependency"));
                } else if !dep_id.is_empty() && !ids.contains(dep_id) {
                    errors.push(format!(
                        "Step '{id}' depends_on '{dep_id}' which does not exist"
                    ));
                }
            }
        }

        // {{variable}} reference check
        for var in extract_template_vars(prompt) {
            // Skip variables that are injected at runtime by the server
            // (memory scopes, RAG context, and the built-in task/args).
            let is_builtin = matches!(
                var.as_str(),
                "task"
                    | "args"
                    | "memory.project"
                    | "memory.user"
                    | "memory.global"
                    | "memory.repo_brain"
                    | "rag_context"
            );
            if !is_builtin && !outputs.contains(var.as_str()) {
                warnings.push(format!(
                    "Step '{id}' references {{{{'{var}'}}}} but no step produces that output key"
                ));
            }
        }
    }

    // Cycle detection via Kahn's topological sort
    let mut in_degree: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut fwd: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for step in steps {
        let id = step.get("id").and_then(|v| v.as_str()).unwrap_or("");
        in_degree.entry(id).or_insert(0);
        fwd.entry(id).or_default();
        if let Some(deps) = step.get("depends_on").and_then(|d| d.as_array()) {
            for dep in deps {
                let dep_id = dep.as_str().unwrap_or("");
                if !dep_id.is_empty() && ids.contains(dep_id) && dep_id != id {
                    fwd.entry(dep_id).or_default().push(id);
                    *in_degree.entry(id).or_insert(0) += 1;
                }
            }
        }
    }
    let mut queue: std::collections::VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, &v)| v == 0)
        .map(|(&k, _)| k)
        .collect();
    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        for &next in fwd.get(node).map(|v| v.as_slice()).unwrap_or(&[]) {
            let deg = in_degree.entry(next).or_insert(0);
            *deg -= 1;
            if *deg == 0 {
                queue.push_back(next);
            }
        }
    }
    if visited < ids.len() {
        errors.push("Circular dependency detected — workflow contains a cycle".into());
    }

    ValidateStructural { errors, warnings }
}

fn extract_template_vars(prompt: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut rest = prompt;
    while let Some(start) = rest.find("{{") {
        rest = &rest[start + 2..];
        if let Some(end) = rest.find("}}") {
            let var = rest[..end].trim().to_string();
            if !var.is_empty() {
                vars.push(var);
            }
            rest = &rest[end + 2..];
        } else {
            break;
        }
    }
    vars
}

async fn validate_with_llm(state: &AppState, workflow: &Value) -> ValidateLlm {
    use agent007_models::types::{CompletionRequest, Message, Role};

    let workflow_json = match serde_json::to_string_pretty(workflow) {
        Ok(s) => s,
        Err(_) => {
            return ValidateLlm {
                available: true,
                score: None,
                summary: Some("Could not serialize workflow".into()),
                issues: vec![],
                suggestions: vec![],
            }
        }
    };

    let prompt = format!(
        "You are a multi-agent workflow validation expert. Review this workflow JSON and assess its logical correctness.\n\n\
         ```json\n{workflow_json}\n```\n\n\
         Check:\n\
         1. Are step prompts well-formed and actionable?\n\
         2. Do {{variable}} references match actual output keys from previous steps?\n\
         3. Is the dependency order logically sound?\n\
         4. Are the agent personas appropriate for their tasks?\n\
         5. Any logical gaps, redundancies, missing steps, or anti-patterns?\n\n\
         Respond with ONLY valid JSON (no markdown fences, no explanation outside JSON):\n\
         {{\"score\": 0-10, \"summary\": \"one sentence\", \"issues\": [\"...\"], \"suggestions\": [\"...\"]}}",
    );

    let request = CompletionRequest {
        model: "default".to_string(),
        messages: vec![Message {
            role: Role::User,
            content: prompt,
        }],
        max_tokens: Some(1024),
        temperature: Some(0.1),
        system: Some("You are a workflow validation expert. Respond only with valid JSON.".into()),
    };

    let provider = state.model_router.route("validation");
    match provider.complete(request).await {
        Err(e) => ValidateLlm {
            available: true,
            score: None,
            summary: Some(format!("LLM validation failed: {e}")),
            issues: vec![],
            suggestions: vec![],
        },
        Ok(resp) => {
            // Strip markdown fences if present
            let content = resp
                .content
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            match serde_json::from_str::<Value>(content) {
                Ok(json) => ValidateLlm {
                    available: true,
                    score: json
                        .get("score")
                        .and_then(|v| v.as_u64())
                        .map(|v| v.min(10) as u8),
                    summary: json
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    issues: json
                        .get("issues")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str())
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default(),
                    suggestions: json
                        .get("suggestions")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str())
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default(),
                },
                Err(_) => ValidateLlm {
                    available: true,
                    score: None,
                    summary: Some(resp.content),
                    issues: vec![],
                    suggestions: vec![],
                },
            }
        }
    }
}

pub async fn workflow_save_handler(
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let name = sanitize_file_stem(
        payload
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("untitled"),
        "untitled",
    );
    let wf_dir = agent007_write_home().join("workflows");
    if let Err(e) = std::fs::create_dir_all(&wf_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let path = wf_dir.join(format!("{name}.yaml"));
    match serde_yaml::to_string(&payload) {
        Ok(yaml) => match std::fs::write(&path, &yaml) {
            Ok(()) => Json(serde_json::json!({
                "ok": true,
                "name": name,
                "path": path.display().to_string()
            }))
            .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Skill save ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SkillSaveRequest {
    pub name: String,
    pub trigger: String,
    pub description: String,
    pub model: Option<String>,
    pub category: Option<String>,
    pub template: String,
}

#[derive(Deserialize)]
pub struct SkillGenerateRequest {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
}

#[derive(Serialize)]
struct SkillFrontmatter<'a> {
    name: &'a str,
    trigger: &'a str,
    description: &'a str,
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'a str>,
}

/// Generate a skill prompt template from the name + description.
/// In hosted-mcp mode (no standalone provider) this returns a well-structured template
/// built from the description without calling an external model.
/// In standalone mode it calls the configured model to write a richer prompt.
pub async fn skill_generate_handler(
    State(state): State<AppState>,
    Json(req): Json<SkillGenerateRequest>,
) -> impl IntoResponse {
    let name = req.name.trim().to_string();
    let description = req.description.trim().to_string();
    let category = req.category.as_deref().unwrap_or("custom");

    // If standalone model is available, ask it to write the prompt.
    if state.standalone_mode {
        let system =
            "You are an expert AI prompt engineer. Write a concise, effective system prompt \
            for an AI skill. Output ONLY the prompt text — no preamble, no markdown fences.";
        let user_msg = format!(
            "Write a prompt template for an AI skill named \"{name}\" that does the following:\n\n\
             {description}\n\n\
             Requirements:\n\
             - Use {{{{args}}}} for the user's input text\n\
             - Use {{{{task}}}} for workflow context (only when inside a workflow)\n\
             - Use {{{{rag_context}}}} if prior knowledge would help\n\
             - Be specific and action-oriented, not generic\n\
             - 100-300 words"
        );
        let request = agent007_models::CompletionRequest {
            model: String::new(),
            messages: vec![agent007_models::Message {
                role: agent007_models::Role::User,
                content: user_msg,
            }],
            max_tokens: Some(600),
            temperature: Some(0.3),
            system: Some(system.to_string()),
        };
        match state.model_router.route(&name).complete(request).await {
            Ok(resp) => {
                return Json(serde_json::json!({ "template": resp.content.trim() }))
                    .into_response();
            }
            Err(_) => {} // fall through to template-based generation
        }
    }

    // Hosted-mcp fallback: build a well-structured template from the description.
    let role_hint = match category {
        "dev" => "senior software engineer",
        "code" => "expert code reviewer",
        "project" => "experienced project manager",
        "meta" => "AI systems specialist",
        _ => "expert AI assistant",
    };

    let template = format!(
        "You are a {role_hint}. {description}\n\n\
         # Task\n{{{{args}}}}\n\n\
         # Context\n{{{{rag_context}}}}\n\n\
         # Instructions\n\
         1. Carefully read the task above.\n\
         2. Apply your expertise as a {role_hint} to produce a thorough, accurate result.\n\
         3. Structure your output clearly with headings or numbered sections.\n\
         4. Be specific and actionable — avoid vague recommendations.\n\
         5. If something is unclear, state your assumptions explicitly.\n\n\
         # Output"
    );

    Json(serde_json::json!({ "template": template })).into_response()
}

pub async fn skill_save_handler(
    State(_state): State<AppState>,
    Json(payload): Json<SkillSaveRequest>,
) -> impl IntoResponse {
    let skills_dir = agent007_write_home().join("skills");
    if let Err(e) = std::fs::create_dir_all(&skills_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    // Use trigger-derived filename so the file is discoverable by trigger lookup.
    let trigger_slug: String = payload
        .trigger
        .trim_start_matches('/')
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let filename = if trigger_slug.is_empty() {
        sanitize_file_stem(&payload.name, "skill")
    } else {
        trigger_slug
    };
    let path = skills_dir.join(format!("{filename}.md"));
    let model = payload.model.as_deref().unwrap_or("codex");
    let category = payload.category.as_deref().filter(|s| !s.is_empty());

    let mut frontmatter_yaml = match serde_yaml::to_string(&SkillFrontmatter {
        name: &payload.name,
        trigger: &payload.trigger,
        description: &payload.description,
        model,
        category,
    }) {
        Ok(yaml) => yaml,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    if let Some(stripped) = frontmatter_yaml.strip_prefix("---\n") {
        frontmatter_yaml = stripped.to_string();
    }
    let content = format!(
        "---\n{}---\n{}\n",
        frontmatter_yaml,
        payload.template.trim_end()
    );

    match std::fs::write(&path, &content) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "path": path.display().to_string() }))
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Memory helpers ────────────────────────────────────────────────────────────

/// Build a MemoryStore rooted at `~/.agent007/memory` (or the project-local equivalent).
/// Map the API "global" scope → empty namespace (files live at the root of the memory dir).
fn memory_store_for_web() -> Arc<agent007_memory::store::MemoryStore> {
    Arc::new(agent007_memory::store::MemoryStore::new(
        agent007_home().join("memory"),
    ))
}

fn web_namespace(scope: &str) -> &str {
    if scope == "global" {
        ""
    } else {
        scope
    }
}

// ── Memory list ───────────────────────────────────────────────────────────────

pub async fn memory_list_handler(
    State(_state): State<AppState>,
    Path(scope): Path<String>,
) -> impl IntoResponse {
    // Basic scope validation — reject traversal attempts
    if scope.contains("..") || scope.contains('/') || scope.contains('\\') {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid scope"})),
        )
            .into_response();
    }
    let store = memory_store_for_web();
    let namespace = web_namespace(&scope);
    let keys = store.scoped(namespace).list_keys().unwrap_or_default();
    Json(keys).into_response()
}

// ── Memory get ────────────────────────────────────────────────────────────────

pub async fn memory_get_handler(
    State(_state): State<AppState>,
    Path((scope, key)): Path<(String, String)>,
) -> impl IntoResponse {
    // Basic scope validation — MemoryStore sanitizes key components internally
    if scope.contains("..") || scope.contains('/') || scope.contains('\\') {
        return (StatusCode::BAD_REQUEST, "invalid scope").into_response();
    }
    // Reject null bytes in key
    if key.contains('\0') {
        return (StatusCode::BAD_REQUEST, "invalid key").into_response();
    }

    let store = memory_store_for_web();
    let namespace = web_namespace(&scope);
    match store.scoped(namespace).read(&key) {
        Ok(Some(content)) => (
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            content,
        )
            .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(e) => {
            tracing::warn!("memory_get scope={scope} key={key}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "read error").into_response()
        }
    }
}

// ── Workflow Templates ────────────────────────────────────────────────────────

pub async fn workflow_templates_list_handler() -> impl IntoResponse {
    Json(get_workflow_templates()).into_response()
}

pub async fn workflow_template_get_handler(Path(name): Path<String>) -> impl IntoResponse {
    let templates = get_workflow_templates();
    match templates
        .iter()
        .find(|t| t.get("name").and_then(|v| v.as_str()) == Some(name.as_str()))
    {
        Some(t) => Json(t.clone()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "template not found" })),
        )
            .into_response(),
    }
}

// ── Skill Registry & Import ──────────────────────────────────────────────────

pub async fn skill_get_handler(
    State(_state): State<AppState>,
    Path(trigger): Path<String>,
) -> impl IntoResponse {
    let target_trigger = format!("/{trigger}");

    // Search project-local first, then global — same order as the list endpoint.
    let mut search_dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(p) = agent007_project_home() {
        search_dirs.push(p.join("skills"));
    }
    search_dirs.push(agent007_global_home().join("skills"));

    for skills_dir in &search_dirs {
        for skill in load_skills_from_dir(skills_dir) {
            if skill.trigger() == target_trigger {
                let mut result = skill_json(&skill, "custom");
                if let Some(obj) = result.as_object_mut() {
                    obj.insert(
                        "template".to_string(),
                        serde_json::Value::String(skill.template().to_string()),
                    );
                }
                return Json(result).into_response();
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "skill not found" })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct SkillImportRequest {
    pub url: String,
}

pub async fn skill_import_handler(
    State(_state): State<AppState>,
    Json(payload): Json<SkillImportRequest>,
) -> impl IntoResponse {
    let source = match parse_skill_import_source(&payload.url) {
        Ok(source) => source,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let client = match reqwest::Client::builder().user_agent("agent007").build() {
        Ok(client) => client,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let imported = match fetch_imported_skill(&client, source, &payload.url).await {
        Ok(imported) => imported,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let skills_dir = agent007_write_home().join("skills");
    let _ = std::fs::create_dir_all(&skills_dir);

    match write_imported_skill(&skills_dir, imported) {
        Ok((trigger, path)) => Json(serde_json::json!({
            "ok": true,
            "trigger": trigger,
            "path": path.display().to_string()
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn skill_registry_handler() -> impl IntoResponse {
    let registry_url =
        "https://raw.githubusercontent.com/danieldear/agent007/main/docs/registry.json";
    let client = reqwest::Client::new();
    match client.get(registry_url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(val) => Json(val).into_response(),
            Err(_) => Json(serde_json::json!([])).into_response(),
        },
        _ => Json(serde_json::json!([])).into_response(),
    }
}

#[derive(Debug)]
enum SkillImportSourceKind {
    DirectFile {
        url: String,
    },
    GitHubFile {
        owner: String,
        repo: String,
        reference: Option<String>,
        path: String,
    },
    GitHubDir {
        owner: String,
        repo: String,
        reference: Option<String>,
        path: String,
    },
}

#[derive(Debug)]
enum ImportedSkillStorage {
    Flat {
        filename: String,
        content: String,
    },
    Package {
        package_name: String,
        files: Vec<(String, String)>,
    },
}

#[derive(Debug)]
struct ImportedSkill {
    trigger: String,
    storage: ImportedSkillStorage,
}

#[derive(Debug, Deserialize)]
struct SkillImportFrontmatter {
    trigger: Option<String>,
    name: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubContentEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    download_url: Option<String>,
}

fn parse_skill_import_source(url: &str) -> anyhow::Result<SkillImportSourceKind> {
    let url = url.trim();

    if url.starts_with("https://raw.githubusercontent.com/")
        || url.starts_with("http://raw.githubusercontent.com/")
    {
        let normalized = url
            .trim_start_matches("https://raw.githubusercontent.com/")
            .trim_start_matches("http://raw.githubusercontent.com/");
        let parts: Vec<&str> = normalized.split('/').collect();
        if parts.len() >= 4 {
            return Ok(SkillImportSourceKind::GitHubFile {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
                reference: Some(parts[2].to_string()),
                path: parts[3..].join("/"),
            });
        }
    }

    if url.contains("github.com") {
        let without_scheme = url
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let parts: Vec<&str> = without_scheme.split('/').collect();
        if parts.len() >= 3 && parts[0] == "github.com" {
            let owner = parts[1].to_string();
            let repo = parts[2].to_string();
            if parts.len() >= 5 && parts[3] == "blob" {
                return Ok(SkillImportSourceKind::GitHubFile {
                    owner,
                    repo,
                    reference: Some(parts[4].to_string()),
                    path: parts[5..].join("/"),
                });
            }
            if parts.len() >= 5 && parts[3] == "tree" {
                return Ok(SkillImportSourceKind::GitHubDir {
                    owner,
                    repo,
                    reference: Some(parts[4].to_string()),
                    path: parts[5..].join("/"),
                });
            }
            return Ok(SkillImportSourceKind::GitHubDir {
                owner,
                repo,
                reference: None,
                path: String::new(),
            });
        }
    }

    if url.starts_with("https://") || url.starts_with("http://") {
        return Ok(SkillImportSourceKind::DirectFile {
            url: url.to_string(),
        });
    }

    Err(anyhow::anyhow!(
        "unsupported source — use a GitHub tree/blob/raw URL or direct https:// skill URL"
    ))
}

async fn fetch_imported_skill(
    client: &reqwest::Client,
    source: SkillImportSourceKind,
    original_url: &str,
) -> anyhow::Result<ImportedSkill> {
    match source {
        SkillImportSourceKind::DirectFile { url } => {
            let content = fetch_text_async(client, &url).await?;
            if content.len() > 100_000 {
                anyhow::bail!("skill file exceeds 100KB limit");
            }
            let slug = fallback_skill_slug_from_url(original_url);
            let (trigger, output) =
                normalize_skill_manifest_content(&content, &slug, original_url)?;
            let filename = format!("{}.md", sanitize_skill_slug(&trigger, &slug));
            Ok(ImportedSkill {
                trigger,
                storage: ImportedSkillStorage::Flat {
                    filename,
                    content: output,
                },
            })
        }
        SkillImportSourceKind::GitHubFile {
            owner,
            repo,
            reference,
            path,
        } => {
            if FsPath::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("SKILL.md"))
                .unwrap_or(false)
            {
                let package_path = FsPath::new(&path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                return fetch_github_package_async(
                    client,
                    &owner,
                    &repo,
                    reference.as_deref(),
                    &package_path,
                    original_url,
                )
                .await;
            }
            let url = github_raw_file_url(&owner, &repo, reference.as_deref(), &path);
            let content = fetch_text_async(client, &url).await?;
            if content.len() > 100_000 {
                anyhow::bail!("skill file exceeds 100KB limit");
            }
            let slug = fallback_skill_slug_from_url(original_url);
            let (trigger, output) =
                normalize_skill_manifest_content(&content, &slug, original_url)?;
            let filename = format!("{}.md", sanitize_skill_slug(&trigger, &slug));
            Ok(ImportedSkill {
                trigger,
                storage: ImportedSkillStorage::Flat {
                    filename,
                    content: output,
                },
            })
        }
        SkillImportSourceKind::GitHubDir {
            owner,
            repo,
            reference,
            path,
        } => {
            fetch_github_package_async(
                client,
                &owner,
                &repo,
                reference.as_deref(),
                &path,
                original_url,
            )
            .await
        }
    }
}

async fn fetch_github_package_async(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    reference: Option<&str>,
    package_path: &str,
    original_url: &str,
) -> anyhow::Result<ImportedSkill> {
    let mut queue = vec![package_path.to_string()];
    let mut files: Vec<(String, String)> = Vec::new();

    while let Some(current_path) = queue.pop() {
        let api_url = github_contents_api_url(owner, repo, reference, &current_path);
        let response =
            client.get(&api_url).send().await.map_err(|e| {
                anyhow::anyhow!("failed to fetch package listing from {api_url}: {e}")
            })?;
        if !response.status().is_success() {
            anyhow::bail!(
                "package listing fetch failed — HTTP {} for {api_url}",
                response.status()
            );
        }
        let entries: Vec<GitHubContentEntry> = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("failed to parse GitHub package listing: {e}"))?;
        for entry in entries {
            match entry.kind.as_str() {
                "file" => {
                    let download_url = entry.download_url.clone().ok_or_else(|| {
                        anyhow::anyhow!("GitHub did not return a download URL for {}", entry.path)
                    })?;
                    let content = fetch_text_async(client, &download_url).await?;
                    let relative = relative_package_path(&entry.path, package_path)?;
                    files.push((relative, content));
                }
                "dir" => queue.push(entry.path),
                _ => {}
            }
        }
    }

    let fallback_slug = fallback_skill_slug_from_url(original_url);
    let skill_index = files
        .iter()
        .position(|(relative, _)| relative.eq_ignore_ascii_case("SKILL.md"))
        .ok_or_else(|| anyhow::anyhow!("package is missing SKILL.md"))?;
    let skill_content = files[skill_index].1.clone();
    let (trigger, normalized_manifest) =
        normalize_skill_manifest_content(&skill_content, &fallback_slug, original_url)?;
    files[skill_index].1 = normalized_manifest;

    Ok(ImportedSkill {
        trigger: trigger.clone(),
        storage: ImportedSkillStorage::Package {
            package_name: sanitize_skill_slug(&trigger, &fallback_slug),
            files,
        },
    })
}

async fn fetch_text_async(client: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("failed to fetch skill from {url}: {e}"))?;
    if !response.status().is_success() {
        anyhow::bail!("fetch failed — HTTP {} for {url}", response.status());
    }
    response
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("failed to read response body: {e}"))
}

fn write_imported_skill(
    skills_dir: &FsPath,
    imported: ImportedSkill,
) -> std::io::Result<(String, PathBuf)> {
    match imported.storage {
        ImportedSkillStorage::Flat { filename, content } => {
            let path = skills_dir.join(filename);
            std::fs::write(&path, content)?;
            Ok((imported.trigger, path))
        }
        ImportedSkillStorage::Package {
            package_name,
            files,
        } => {
            let path = skills_dir.join(&package_name);
            if path.exists() {
                std::fs::remove_dir_all(&path)?;
            }
            std::fs::create_dir_all(&path)?;
            for (relative, content) in files {
                let relative_path = FsPath::new(&relative);
                let dest = path.join(relative_path);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(dest, content)?;
            }
            Ok((imported.trigger, path))
        }
    }
}

fn fallback_skill_slug_from_url(url: &str) -> String {
    let url_parts: Vec<&str> = url.split('/').filter(|s| !s.is_empty()).collect();
    let last = url_parts.last().copied().unwrap_or("imported-skill");
    let last_no_ext = last.trim_end_matches(".md").trim_end_matches(".MD");
    let effective = if matches!(
        last_no_ext.to_lowercase().as_str(),
        "skill" | "skills" | "readme" | "index"
    ) {
        if url_parts.len() >= 2 {
            url_parts[url_parts.len() - 2]
        } else {
            last_no_ext
        }
    } else {
        last_no_ext
    };
    sanitize_skill_slug(effective, "imported-skill")
}

fn normalize_skill_manifest_content(
    content: &str,
    fallback_slug: &str,
    original_url: &str,
) -> anyhow::Result<(String, String)> {
    let url_trigger = format!("/{fallback_slug}");
    let url_name: String = fallback_slug
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let url_description = format!("Imported from {original_url}");

    let parts: Vec<&str> = content.splitn(3, "---").collect();
    let (fm, body, has_original_trigger) = if parts.len() >= 3 {
        match serde_yaml::from_str::<SkillImportFrontmatter>(parts[1]) {
            Ok(f) => {
                let has_trigger = f.trigger.is_some();
                (f, parts[2].to_string(), has_trigger)
            }
            Err(_) => (
                SkillImportFrontmatter {
                    trigger: None,
                    name: None,
                    description: None,
                },
                content.to_string(),
                false,
            ),
        }
    } else {
        (
            SkillImportFrontmatter {
                trigger: None,
                name: None,
                description: None,
            },
            content.to_string(),
            false,
        )
    };

    let trigger = fm.trigger.unwrap_or_else(|| url_trigger.clone());
    let name = fm.name.unwrap_or(url_name);
    let description = fm.description.unwrap_or(url_description);

    let output = if has_original_trigger {
        content.to_string()
    } else {
        format!("---\nname: {name}\ntrigger: {trigger}\ndescription: {description}\n---\n{body}")
    };

    Ok((trigger, output))
}

fn sanitize_skill_slug(value: &str, fallback: &str) -> String {
    let slug: String = value
        .trim_start_matches('/')
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug.to_string()
    }
}

fn github_raw_file_url(owner: &str, repo: &str, reference: Option<&str>, path: &str) -> String {
    let reference = reference.unwrap_or("HEAD");
    format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{reference}/{}",
        path.trim_start_matches('/')
    )
}

fn github_contents_api_url(owner: &str, repo: &str, reference: Option<&str>, path: &str) -> String {
    let mut url = if path.trim().is_empty() {
        format!("https://api.github.com/repos/{owner}/{repo}/contents")
    } else {
        format!(
            "https://api.github.com/repos/{owner}/{repo}/contents/{}",
            path.trim_start_matches('/')
        )
    };
    if let Some(reference) = reference {
        url.push_str(&format!("?ref={reference}"));
    }
    url
}

fn relative_package_path(entry_path: &str, package_root: &str) -> anyhow::Result<String> {
    if package_root.trim().is_empty() {
        return Ok(entry_path.to_string());
    }
    FsPath::new(entry_path)
        .strip_prefix(FsPath::new(package_root))
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|_| anyhow::anyhow!("failed to derive relative path for {entry_path}"))
}

fn get_workflow_templates() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "pipeline",
            "description": "Sequential chain: each step feeds into the next",
            "steps": [
                { "id": "research", "agent": "Researcher", "prompt": "Research best practices for: {{task}}", "output": "research_notes" },
                { "id": "design", "agent": "Architect", "prompt": "Design based on: {{research_notes}}", "output": "plan", "depends_on": ["research"] },
                { "id": "implement", "agent": "Coder", "prompt": "Implement: {{plan}}", "output": "code", "depends_on": ["design"] }
            ]
        }),
        serde_json::json!({
            "name": "fan-out",
            "description": "Split work to parallel agents, then merge results",
            "steps": [
                { "id": "split", "agent": "Architect", "prompt": "Break down into independent concerns: {{task}}", "output": "concerns" },
                { "id": "security-review", "agent": "SecurityReviewer", "prompt": "Security analysis: {{concerns}}", "output": "security_report", "depends_on": ["split"] },
                { "id": "performance-review", "agent": "PerformanceEngineer", "prompt": "Performance analysis: {{concerns}}", "output": "perf_report", "depends_on": ["split"] },
                { "id": "style-review", "agent": "CodeReviewer", "prompt": "Style review: {{concerns}}", "output": "style_report", "depends_on": ["split"] },
                { "id": "merge", "agent": "Architect", "prompt": "Synthesize findings: {{security_report}} {{perf_report}} {{style_report}}", "output": "final_report", "depends_on": ["security-review", "performance-review", "style-review"] }
            ]
        }),
        serde_json::json!({
            "name": "hierarchical",
            "description": "Coordinator delegates to specialist sub-agents",
            "steps": [
                { "id": "plan", "agent": "Architect", "prompt": "Break into frontend, backend, infra tasks: {{task}}", "output": "breakdown" },
                { "id": "frontend", "agent": "UIUXDesigner", "prompt": "Implement UI: {{breakdown}}", "output": "ui_code", "depends_on": ["plan"] },
                { "id": "backend", "agent": "Coder", "prompt": "Implement API: {{breakdown}}", "output": "api_code", "depends_on": ["plan"] },
                { "id": "infra", "agent": "DevOpsEngineer", "prompt": "Setup infra: {{breakdown}}", "output": "infra_config", "depends_on": ["plan"] },
                { "id": "integrate", "agent": "Architect", "prompt": "Integrate: {{ui_code}} {{api_code}} {{infra_config}}", "output": "integrated", "depends_on": ["frontend", "backend", "infra"] }
            ]
        }),
        serde_json::json!({
            "name": "review-loop",
            "description": "Implement, review, retry until quality passes",
            "steps": [
                { "id": "implement", "agent": "Coder", "prompt": "Implement {{task}}. Previous feedback: {{review_result}}", "output": "code" },
                { "id": "review", "agent": "CodeReviewer", "type": "evaluator", "prompt": "Review code quality: {{code}}. Respond with JSON: {\"verdict\": \"pass\" or \"retry\", \"reason\": \"...\"}", "output": "review_result", "depends_on": ["implement"], "evaluate": { "decision_field": "verdict", "on_pass": "deploy", "on_fail": "implement", "max_retries": 3 } },
                { "id": "deploy", "agent": "DevOpsEngineer", "prompt": "Deploy verified code: {{code}}", "output": "deployment", "depends_on": ["review"] }
            ]
        }),
        serde_json::json!({
            "name": "router",
            "description": "Classify task and route to the right specialist",
            "steps": [
                { "id": "classify", "agent": "Researcher", "type": "router", "prompt": "Classify this task. Respond with one of: frontend, backend, infra. Task: {{task}}", "output": "classification", "routes": [{ "when": "frontend", "goto": "ui-work" }, { "when": "backend", "goto": "api-work" }, { "goto": "infra-work", "default": true }] },
                { "id": "ui-work", "agent": "UIUXDesigner", "prompt": "Handle frontend task: {{task}}", "output": "result" },
                { "id": "api-work", "agent": "Coder", "prompt": "Handle backend task: {{task}}", "output": "result" },
                { "id": "infra-work", "agent": "DevOpsEngineer", "prompt": "Handle infra task: {{task}}", "output": "result" },
                { "id": "summarize", "agent": "Researcher", "prompt": "Summarize outcome: {{result}}", "output": "summary", "depends_on": ["ui-work", "api-work", "infra-work"] }
            ]
        }),
        serde_json::json!({
            "name": "orchestrator",
            "description": "Master orchestrator decomposes goal, delegates to specialists, synthesizes final result",
            "steps": [
                { "id": "decompose", "agent": "Architect", "prompt": "You are a master orchestrator. Decompose this goal into 3-5 concrete subtasks, each assignable to a specialist agent. Goal: {{task}}\n\nOutput a numbered list of subtasks with the agent best suited for each.", "output": "subtasks" },
                { "id": "research", "agent": "Researcher", "prompt": "Execute your assigned subtask from this plan:\n{{subtasks}}\n\nYour role: Researcher — gather context, facts, and prior art.", "output": "research_output", "depends_on": ["decompose"] },
                { "id": "design", "agent": "Architect", "prompt": "Execute your assigned subtask from this plan:\n{{subtasks}}\n\nResearch context: {{research_output}}\n\nYour role: Architect — produce the structural design.", "output": "design_output", "depends_on": ["research"] },
                { "id": "implement", "agent": "Coder", "prompt": "Execute your assigned subtask from this plan:\n{{subtasks}}\n\nDesign: {{design_output}}\n\nYour role: Coder — write the implementation.", "output": "impl_output", "depends_on": ["design"] },
                { "id": "validate", "agent": "CodeReviewer", "type": "evaluator", "prompt": "Validate the implementation against the original goal.\nGoal: {{task}}\nImplementation: {{impl_output}}\n\nRespond JSON: {\"verdict\": \"pass\" or \"retry\", \"gaps\": \"...\", \"fixes\": \"...\"}", "output": "validation", "depends_on": ["implement"], "evaluate": { "decision_field": "verdict", "on_pass": "synthesize", "on_fail": "implement", "max_retries": 2 } },
                { "id": "synthesize", "agent": "Architect", "prompt": "Synthesize all agent outputs into a final deliverable.\n\nSubtasks: {{subtasks}}\nResearch: {{research_output}}\nDesign: {{design_output}}\nImplementation: {{impl_output}}\nValidation: {{validation}}\n\nProduce a cohesive final report with executive summary, key decisions, and next steps.", "output": "final_result", "depends_on": ["validate"] }
            ]
        }),
        serde_json::json!({
            "name": "map-reduce",
            "description": "Split input into chunks, process each in parallel, reduce to final output",
            "steps": [
                { "id": "map-split", "agent": "Architect", "prompt": "Split this task into 4 independent, equal-sized chunks that can be processed in parallel. Each chunk should be self-contained.\n\nTask: {{task}}\n\nOutput 4 clearly labelled chunks.", "output": "chunks" },
                { "id": "map-1", "agent": "Researcher", "prompt": "Process chunk 1 from:\n{{chunks}}\n\nDeliver a complete analysis of your assigned chunk only.", "output": "chunk1_result", "depends_on": ["map-split"] },
                { "id": "map-2", "agent": "Coder", "prompt": "Process chunk 2 from:\n{{chunks}}\n\nDeliver a complete analysis of your assigned chunk only.", "output": "chunk2_result", "depends_on": ["map-split"] },
                { "id": "map-3", "agent": "SecurityReviewer", "prompt": "Process chunk 3 from:\n{{chunks}}\n\nDeliver a complete analysis of your assigned chunk only.", "output": "chunk3_result", "depends_on": ["map-split"] },
                { "id": "map-4", "agent": "PerformanceEngineer", "prompt": "Process chunk 4 from:\n{{chunks}}\n\nDeliver a complete analysis of your assigned chunk only.", "output": "chunk4_result", "depends_on": ["map-split"] },
                { "id": "reduce", "agent": "Architect", "prompt": "Reduce all chunk results into a single unified output. Remove duplicates, resolve conflicts, and produce a coherent whole.\n\nChunk 1: {{chunk1_result}}\nChunk 2: {{chunk2_result}}\nChunk 3: {{chunk3_result}}\nChunk 4: {{chunk4_result}}", "output": "reduced_result", "depends_on": ["map-1", "map-2", "map-3", "map-4"] }
            ]
        }),
        serde_json::json!({
            "name": "consensus",
            "description": "Multiple agents independently analyze, then vote — majority position wins",
            "steps": [
                { "id": "agent-a", "agent": "Researcher", "prompt": "Independently analyze this without seeing other agents' views.\n\nTask: {{task}}\n\nProvide your assessment, recommendation, and confidence (high/medium/low).", "output": "view_a" },
                { "id": "agent-b", "agent": "Architect", "prompt": "Independently analyze this without seeing other agents' views.\n\nTask: {{task}}\n\nProvide your assessment, recommendation, and confidence (high/medium/low).", "output": "view_b" },
                { "id": "agent-c", "agent": "CodeReviewer", "prompt": "Independently analyze this without seeing other agents' views.\n\nTask: {{task}}\n\nProvide your assessment, recommendation, and confidence (high/medium/low).", "output": "view_c" },
                { "id": "vote", "agent": "Architect", "prompt": "You are a consensus judge. Review three independent agent assessments and determine the majority position.\n\nAgent A: {{view_a}}\nAgent B: {{view_b}}\nAgent C: {{view_c}}\n\nIdentify: (1) points of agreement, (2) points of conflict, (3) the majority consensus recommendation, (4) minority dissent worth noting.", "output": "consensus_result", "depends_on": ["agent-a", "agent-b", "agent-c"] }
            ]
        }),
        serde_json::json!({
            "name": "debate",
            "description": "Two agents argue opposing positions, a judge evaluates and decides",
            "steps": [
                { "id": "frame", "agent": "Architect", "prompt": "Frame the following as a debate with two clearly opposing positions (e.g. approach A vs approach B, build vs buy, SQL vs NoSQL).\n\nTopic: {{task}}\n\nOutput: Position 1 (title + core argument) and Position 2 (title + core argument).", "output": "debate_frame" },
                { "id": "argue-for", "agent": "Researcher", "prompt": "You are arguing FOR Position 1 in this debate:\n{{debate_frame}}\n\nBuild the strongest possible case for Position 1. Use evidence, examples, and logical arguments. Anticipate and pre-refute the strongest objections.", "output": "argument_for" },
                { "id": "argue-against", "agent": "SecurityReviewer", "prompt": "You are arguing FOR Position 2 in this debate:\n{{debate_frame}}\n\nBuild the strongest possible case for Position 2. Use evidence, examples, and logical arguments. Anticipate and pre-refute the strongest objections.", "output": "argument_against" },
                { "id": "rebut-for", "agent": "Researcher", "prompt": "Read the opposing argument and provide a focused rebuttal.\n\nYour position (Position 1): {{argument_for}}\nOpposing argument (Position 2): {{argument_against}}\n\nRebuttal: address their strongest points directly.", "output": "rebuttal_for", "depends_on": ["argue-for", "argue-against"] },
                { "id": "rebut-against", "agent": "SecurityReviewer", "prompt": "Read the opposing argument and provide a focused rebuttal.\n\nYour position (Position 2): {{argument_against}}\nOpposing argument (Position 1): {{argument_for}}\n\nRebuttal: address their strongest points directly.", "output": "rebuttal_against", "depends_on": ["argue-for", "argue-against"] },
                { "id": "judge", "agent": "Architect", "prompt": "You are the debate judge. Evaluate both sides fairly and reach a verdict.\n\nDebate topic: {{debate_frame}}\n\nPosition 1 argued: {{argument_for}}\nPosition 1 rebuttal: {{rebuttal_for}}\nPosition 2 argued: {{argument_against}}\nPosition 2 rebuttal: {{rebuttal_against}}\n\nVerdict: which position is stronger and why? What is the recommended course of action?", "output": "verdict", "depends_on": ["rebut-for", "rebut-against"] }
            ]
        }),
    ]
}

// ── Promote to global ─────────────────────────────────────────────────────────

/// `POST /api/skills/{trigger}/promote` — copy a project-local skill to ~/.agent007/skills/.
pub async fn skill_promote_handler(
    State(_state): State<AppState>,
    Path(trigger): Path<String>,
) -> impl IntoResponse {
    let target_trigger = format!("/{}", trigger.trim_start_matches('/'));

    let project_skills = match agent007_project_home() {
        Some(p) => p.join("skills"),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "no project-local .agent007 found" })),
            )
                .into_response()
        }
    };
    let global_skills = agent007_global_home().join("skills");

    let Some(skill) = load_skills_from_dir(&project_skills)
        .into_iter()
        .find(|skill| skill.trigger() == target_trigger)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "skill not found in project" })),
        )
            .into_response();
    };

    let _ = std::fs::create_dir_all(&global_skills);
    if let Some(existing) = load_skills_from_dir(&global_skills)
        .into_iter()
        .find(|existing| existing.trigger() == target_trigger)
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "skill already exists globally",
                "path": existing.entry_path().display().to_string()
            })),
        )
            .into_response();
    }

    let dest = destination_entry_path(&skill, &global_skills);
    match copy_skill_entry_to_dir(&skill, &global_skills) {
        Ok(_) => {
            // Remove the project-local copy so the skill no longer appears as PROJ
            let _ = remove_skill_entry(&skill);
            Json(serde_json::json!({
                "ok": true,
                "promoted_to": dest.display().to_string()
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/workflows/{name}/promote` — copy a project-local workflow to ~/.agent007/workflows/.
pub async fn workflow_promote_handler(
    State(_state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let requested = name.trim();
    let safe_name = sanitize_file_stem(requested, "");
    if safe_name.is_empty() || safe_name != requested {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid workflow name" })),
        )
            .into_response();
    }

    let project_wf = agent007_project_home().map(|p| p.join("workflows"));
    let global_wf = agent007_global_home().join("workflows");

    let src = project_wf.and_then(|dir| {
        [
            dir.join(format!("{safe_name}.yaml")),
            dir.join(format!("{safe_name}.yml")),
        ]
        .into_iter()
        .find(|p| p.exists())
    });

    let Some(src) = src else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "workflow not found in project" })),
        )
            .into_response();
    };

    let Some(filename) = src.file_name().map(|f| f.to_string_lossy().to_string()) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "invalid workflow filename" })),
        )
            .into_response();
    };
    let _ = std::fs::create_dir_all(&global_wf);
    let dest = global_wf.join(&filename);

    if dest.exists() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "workflow already exists globally",
                "path": dest.display().to_string()
            })),
        )
            .into_response();
    }

    match std::fs::copy(&src, &dest) {
        Ok(_) => {
            // Remove the project-local copy so the workflow no longer appears as PROJ
            let _ = std::fs::remove_file(&src);
            Json(serde_json::json!({
                "ok": true,
                "promoted_to": dest.display().to_string()
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Bundle export / import ─────────────────────────────────────────────────────

/// `DELETE /api/workflows/:name` — delete a workflow file by name.
pub async fn workflow_delete_handler(Path(name): Path<String>) -> impl IntoResponse {
    let requested = name.trim();
    let safe_name = sanitize_file_stem(requested, "");
    if safe_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid workflow name" })),
        )
            .into_response();
    }

    // Search project-local first, then global
    let candidates: Vec<std::path::PathBuf> = {
        let mut v = Vec::new();
        if let Some(p) = agent007_project_home() {
            let dir = p.join("workflows");
            v.push(dir.join(format!("{safe_name}.yaml")));
            v.push(dir.join(format!("{safe_name}.yml")));
        }
        let global = agent007_global_home().join("workflows");
        v.push(global.join(format!("{safe_name}.yaml")));
        v.push(global.join(format!("{safe_name}.yml")));
        v
    };

    for path in &candidates {
        if path.exists() {
            return match std::fs::remove_file(path) {
                Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response(),
            };
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "workflow not found" })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct BundleExportQuery {
    pub skills: Option<String>,
    pub workflows: Option<String>,
}

/// `GET /api/bundle/export` — export selected (or all) skills+workflows as JSON bundle.
pub async fn bundle_export_handler(
    State(_state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<BundleExportQuery>,
) -> impl IntoResponse {
    let skills_dir = agent007_home().join("skills");
    let workflows_dir = agent007_home().join("workflows");

    let skill_filters: Vec<&str> = params
        .skills
        .as_deref()
        .map(|s| s.split(',').collect())
        .unwrap_or_default();
    let wf_filters: Vec<&str> = params
        .workflows
        .as_deref()
        .map(|s| s.split(',').collect())
        .unwrap_or_default();

    let builder = agent007_sharing::BundleBuilder::new(&skills_dir, &workflows_dir);
    match builder.build(&skill_filters, &wf_filters) {
        Ok(bundle) => match bundle.to_json() {
            Ok(json) => (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, "application/json"),
                    (
                        axum::http::header::CONTENT_DISPOSITION,
                        "attachment; filename=\"agent007-bundle.a7bundle\"",
                    ),
                ],
                json,
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct BundleImportRequest {
    pub bundle: serde_json::Value,
    #[serde(default)]
    pub overwrite: bool,
}

/// `POST /api/bundle/import` — import a bundle JSON into the current project.
pub async fn bundle_import_handler(
    State(_state): State<AppState>,
    Json(payload): Json<BundleImportRequest>,
) -> impl IntoResponse {
    let bundle_json = match serde_json::to_string(&payload.bundle) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let bundle = match agent007_sharing::Bundle::from_json(&bundle_json) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid bundle: {e}") })),
            )
                .into_response()
        }
    };

    let skills_dir = agent007_write_home().join("skills");
    let workflows_dir = agent007_write_home().join("workflows");
    let importer = agent007_sharing::BundleImporter::new(&skills_dir, &workflows_dir);

    match importer.import(&bundle, payload.overwrite) {
        Ok(results) => {
            let imported = results
                .iter()
                .filter(|r| r.action == agent007_sharing::ImportAction::Imported)
                .count();
            let skipped = results
                .iter()
                .filter(|r| r.action == agent007_sharing::ImportAction::Skipped)
                .count();
            let overwritten = results
                .iter()
                .filter(|r| r.action == agent007_sharing::ImportAction::Overwritten)
                .count();
            Json(serde_json::json!({
                "ok": true,
                "results": results,
                "imported": imported,
                "skipped": skipped,
                "overwritten": overwritten,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn load_skills_from_dir(dir: &FsPath) -> Vec<agent007_skills::Skill> {
    if !dir.exists() {
        return Vec::new();
    }
    let loader = agent007_skills::SkillLoader::new(dir);
    loader.load_all().unwrap_or_default()
}

fn skill_json(skill: &agent007_skills::Skill, source: &str) -> Value {
    serde_json::json!({
        "trigger": skill.trigger(),
        "name": skill.name(),
        "description": skill.frontmatter.description,
        "category": skill.category(),
        "version": skill.version(),
        "tags": skill.tags(),
        "model": skill.model(),
        "source": source,
        "format": if skill.is_package() { "package" } else { "flat" },
        "path": skill.entry_path().display().to_string(),
    })
}

fn remove_skill_entry(skill: &agent007_skills::Skill) -> std::io::Result<()> {
    if skill.is_package() {
        std::fs::remove_dir_all(skill.entry_path())
    } else {
        std::fs::remove_file(skill.entry_path())
    }
}

fn destination_entry_path(skill: &agent007_skills::Skill, dest_dir: &FsPath) -> PathBuf {
    let fallback = skill.trigger().trim_start_matches('/').replace('/', "-");
    let name = skill
        .entry_path()
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback.as_str());
    dest_dir.join(name)
}

fn copy_skill_entry_to_dir(
    skill: &agent007_skills::Skill,
    dest_dir: &FsPath,
) -> std::io::Result<()> {
    let dest = destination_entry_path(skill, dest_dir);
    if skill.is_package() {
        copy_dir_recursive(skill.entry_path(), &dest)
    } else {
        std::fs::copy(skill.entry_path(), dest).map(|_| ())
    }
}

fn copy_dir_recursive(src: &FsPath, dest: &FsPath) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_recursive(&entry_path, &dest_path)?;
        } else {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&entry_path, &dest_path)?;
        }
    }
    Ok(())
}

/// No-op VectorDB for dry-run skill execution.
struct NoOpVectorDB;

#[async_trait::async_trait]
impl agent007_memory::VectorDB for NoOpVectorDB {
    async fn upsert(
        &self,
        _id: &str,
        _vector: Vec<f32>,
        _payload: serde_json::Value,
    ) -> Result<(), agent007_memory::MemoryError> {
        Ok(())
    }

    async fn search(
        &self,
        _query: Vec<f32>,
        _limit: usize,
    ) -> Result<Vec<agent007_memory::SearchResult>, agent007_memory::MemoryError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::EXTERNAL_WORKFLOW_CONTROL_ERROR;
    use crate::server::WebServer;
    use axum::http::StatusCode;
    use axum_test::TestServer;

    fn test_server() -> TestServer {
        let server = WebServer::new_test();
        TestServer::new(server.into_router()).unwrap()
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    async fn api_run_accepts_task() {
        let _guard = ENV_LOCK.lock().unwrap();
        let ts = test_server();
        let response = ts
            .post("/api/run")
            .json(&serde_json::json!({ "task": "hello" }))
            .await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(
            body.get("message").and_then(|value| value.as_str()),
            Some("test")
        );
        let session = body
            .get("session")
            .and_then(|value| value.as_str())
            .expect("session should be present");

        let detail = ts.get(&format!("/api/runs/{session}")).await;
        detail.assert_status_ok();
        let detail_body: serde_json::Value = detail.json();
        assert_eq!(
            detail_body
                .get("output_text")
                .and_then(|value| value.as_str()),
            Some("test")
        );
        assert_eq!(
            detail_body["run"]["metadata"]["output_preview"].as_str(),
            Some("test")
        );
        assert_eq!(
            detail_body["run"]["metadata"]["provider"].as_str(),
            Some("mock")
        );
        assert_eq!(
            detail_body["run"]["entries"]
                .as_array()
                .map(|entries| entries.len()),
            Some(2)
        );
    }

    #[tokio::test]
    async fn api_skills_returns_array() {
        let ts = test_server();
        let response = ts.get("/api/skills").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body.is_array());
    }

    #[tokio::test]
    async fn api_status_returns_object() {
        let ts = test_server();
        let response = ts.get("/api/status").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body.is_object());
        assert!(body.get("agents").is_some());
        assert!(body.get("tasks").is_some());
        assert!(body.get("avg_reward").is_some());
    }

    #[tokio::test]
    async fn api_workflow_templates_returns_array() {
        let ts = test_server();
        let response = ts.get("/api/workflow-templates").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body.is_array());
        let arr = body.as_array().unwrap();
        assert!(arr.len() >= 5);
    }

    #[tokio::test]
    async fn api_workflow_template_get_returns_template() {
        let ts = test_server();
        let response = ts.get("/api/workflow-templates/pipeline").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body.get("name").unwrap().as_str(), Some("pipeline"));
    }

    #[tokio::test]
    async fn api_runs_returns_array() {
        let ts = test_server();
        let response = ts.get("/api/runs").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body.is_array());
    }

    #[tokio::test]
    async fn api_stats_uses_shared_run_store_snapshot() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions").join("session-1");
        std::fs::create_dir_all(&sessions).unwrap();
        std::env::set_var("AGENT007_HOME", home.path());
        let prompt_ref = serde_json::to_value(agent007_core::types::PromptRef::new()).unwrap();
        std::fs::write(
            sessions.join("meta.json"),
            serde_json::json!({
                "id": "session-1",
                "kind": "task",
                "task": "ship auth",
                "mode": "hosted-mcp",
                "provider": "codex",
                "started_at": chrono::Utc::now(),
                "finished_at": chrono::Utc::now(),
                "status": "succeeded",
                "output_preview": "done"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            sessions.join("events.jsonl"),
            serde_json::json!({
                "timestamp": chrono::Utc::now(),
                "kind": "agent-event",
                "payload": {
                    "ModelRequest": {
                        "provider": "codex",
                        "prompt_ref": prompt_ref,
                        "token_estimate": 222
                    }
                }
            })
            .to_string()
                + "\n",
        )
        .unwrap();

        let ts = test_server();
        let response = ts.get("/api/stats").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(
            body.get("completed_tasks").and_then(|value| value.as_u64()),
            Some(1)
        );
        assert_eq!(
            body.get("session_requests")
                .and_then(|value| value.as_u64()),
            Some(1)
        );
        assert_eq!(
            body.get("total_tokens").and_then(|value| value.as_u64()),
            Some(222)
        );

        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn api_scorecards_returns_recent_scorecards() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions").join("session-1");
        std::fs::create_dir_all(&sessions).unwrap();
        std::env::set_var("AGENT007_HOME", home.path());

        let now = chrono::Utc::now();
        std::fs::write(
            sessions.join("meta.json"),
            serde_json::json!({
                "id": "session-1",
                "kind": "workflow",
                "task": "ship scorecards",
                "mode": "hosted-mcp",
                "provider": "codex",
                "started_at": now,
                "finished_at": now,
                "status": "succeeded",
                "output_preview": "done"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            sessions.join("run-scorecard.json"),
            serde_json::json!({
                "schema_version": 1,
                "run_id": "session-1",
                "kind": "workflow",
                "mode": "hosted-mcp",
                "provider": "codex",
                "status": "succeeded",
                "completed": true,
                "success": true,
                "started_at": now,
                "finished_at": now,
                "duration_ms": 1500,
                "tokens": 1000,
                "requests": 1,
                "estimated_usd": 0.002,
                "retry_count": 0,
                "tool_calls": 0,
                "tool_errors": 0,
                "quality_score": 99.0,
                "updated_at": now
            })
            .to_string(),
        )
        .unwrap();

        let ts = test_server();
        let response = ts.get("/api/scorecards?limit=5").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body.is_array());
        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(
            arr[0].get("run_id").and_then(|v| v.as_str()),
            Some("session-1")
        );

        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn api_regression_evaluate_reports_threshold_failures() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", home.path());

        let now = chrono::Utc::now();
        for (id, success, cost, retries) in [
            ("session-1", true, 0.4, 1u32),
            ("session-2", false, 1.2, 4u32),
        ] {
            let session_dir = home.path().join("sessions").join(id);
            std::fs::create_dir_all(&session_dir).unwrap();
            std::fs::write(
                session_dir.join("meta.json"),
                serde_json::json!({
                    "id": id,
                    "kind": "workflow",
                    "task": "regression sample",
                    "mode": "hosted-mcp",
                    "provider": "codex",
                    "started_at": now,
                    "finished_at": now,
                    "status": if success { "succeeded" } else { "failed" },
                    "output_preview": "done"
                })
                .to_string(),
            )
            .unwrap();
            std::fs::write(
                session_dir.join("run-scorecard.json"),
                serde_json::json!({
                    "schema_version": 1,
                    "run_id": id,
                    "kind": "workflow",
                    "mode": "hosted-mcp",
                    "provider": "codex",
                    "status": if success { "succeeded" } else { "failed" },
                    "completed": true,
                    "success": success,
                    "started_at": now,
                    "finished_at": now,
                    "duration_ms": 10_000,
                    "tokens": 2_000,
                    "requests": 1,
                    "estimated_usd": cost,
                    "retry_count": retries,
                    "tool_calls": 1,
                    "tool_errors": if success { 0 } else { 1 },
                    "quality_score": if success { 90.0 } else { 20.0 },
                    "updated_at": now
                })
                .to_string(),
            )
            .unwrap();
        }

        let ts = test_server();
        let response = ts
            .get("/api/regression/evaluate?min_success_rate=0.8&max_avg_cost_usd=0.5&max_avg_retries=1.5")
            .await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body.get("passed").and_then(|v| v.as_bool()), Some(false));
        assert!(body
            .get("violations")
            .and_then(|v| v.as_array())
            .map(|items| !items.is_empty())
            .unwrap_or(false));

        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn api_run_approval_records_decision() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions").join("session-1");
        std::fs::create_dir_all(&sessions).unwrap();
        std::env::set_var("AGENT007_HOME", home.path());
        std::fs::write(
            sessions.join("meta.json"),
            serde_json::json!({
                "id": "session-1",
                "kind": "workflow-web-resume",
                "task": "approve auth",
                "mode": "standalone",
                "provider": "mock",
                "started_at": chrono::Utc::now(),
                "finished_at": null,
                "status": "awaiting-approval",
                "output_preview": "approval required"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            sessions.join("workflow-state.json"),
            serde_json::json!({
                "workflow": "approval-flow",
                "task": "approve auth",
                "status": "waiting-approval",
                "steps_total": 1,
                "steps_completed": 0,
                "completed_steps": [],
                "skipped_steps": [],
                "retry_counts": {},
                "outputs": {},
                "budget_used": { "tokens": 0, "estimated_usd": 0.0 },
                "steps": [{
                    "id": "approve-me",
                    "agent": "Architect",
                    "status": "awaiting-approval",
                    "attempts": 1,
                    "output_key": "plan",
                    "output_preview": "draft plan",
                    "selected_route": null,
                    "selected_target": null,
                    "error": null
                }],
                "pending_approval": {
                    "step_id": "approve-me",
                    "agent": "Architect",
                    "output_key": "plan",
                    "content": "draft plan",
                    "content_preview": "draft plan"
                },
                "approval_decisions": {},
                "last_error": null
            })
            .to_string(),
        )
        .unwrap();

        let ts = test_server();
        let response = ts
            .post("/api/runs/session-1/approval")
            .json(&serde_json::json!({ "decision": "approve" }))
            .await;
        response.assert_status_ok();

        let state: agent007_workflows::WorkflowRunState = serde_json::from_str(
            &std::fs::read_to_string(sessions.join("workflow-state.json")).unwrap(),
        )
        .unwrap();
        assert!(state.pending_approval.is_none());
        assert!(state.approval_decisions.contains_key("approve-me"));
        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn api_run_approval_rejects_external_workflow_run() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions").join("session-1");
        std::fs::create_dir_all(&sessions).unwrap();
        std::env::set_var("AGENT007_HOME", home.path());
        std::fs::write(
            sessions.join("meta.json"),
            serde_json::json!({
                "id": "session-1",
                "kind": "workflow",
                "task": "approve auth",
                "mode": "standalone",
                "provider": "mock",
                "started_at": chrono::Utc::now(),
                "finished_at": null,
                "status": "awaiting-approval",
                "output_preview": "approval required"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            sessions.join("workflow-state.json"),
            serde_json::json!({
                "workflow": "approval-flow",
                "task": "approve auth",
                "status": "waiting-approval",
                "steps_total": 1,
                "steps_completed": 0,
                "completed_steps": [],
                "skipped_steps": [],
                "retry_counts": {},
                "outputs": {},
                "budget_used": { "tokens": 0, "estimated_usd": 0.0 },
                "steps": [{
                    "id": "approve-me",
                    "agent": "Architect",
                    "status": "awaiting-approval",
                    "attempts": 1,
                    "output_key": "plan",
                    "output_preview": "draft plan",
                    "selected_route": null,
                    "selected_target": null,
                    "error": null
                }],
                "pending_approval": {
                    "step_id": "approve-me",
                    "agent": "Architect",
                    "output_key": "plan",
                    "content": "draft plan",
                    "content_preview": "draft plan"
                },
                "approval_decisions": {},
                "last_error": null
            })
            .to_string(),
        )
        .unwrap();

        let ts = test_server();
        let response = ts
            .post("/api/runs/session-1/approval")
            .json(&serde_json::json!({ "decision": "approve" }))
            .await;
        response.assert_status(StatusCode::CONFLICT);
        let body: serde_json::Value = response.json();
        assert_eq!(
            body.get("error").and_then(|value| value.as_str()),
            Some(EXTERNAL_WORKFLOW_CONTROL_ERROR)
        );

        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn api_run_resume_creates_new_session() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions").join("session-1");
        let workflows = home.path().join("workflows");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&workflows).unwrap();
        std::env::set_var("AGENT007_HOME", home.path());

        std::fs::write(
            workflows.join("approval-flow.toml"),
            r#"
name = "approval-flow"

[[steps]]
id = "approve-me"
agent = "Architect"
prompt = "Plan {{task}}"
output = "plan"
requires_approval = true
"#,
        )
        .unwrap();
        std::fs::write(
            sessions.join("meta.json"),
            serde_json::json!({
                "id": "session-1",
                "kind": "workflow-web-resume",
                "task": "approve auth",
                "mode": "standalone",
                "provider": "mock",
                "started_at": chrono::Utc::now(),
                "finished_at": chrono::Utc::now(),
                "status": "awaiting-approval",
                "output_preview": "approval required"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            sessions.join("workflow-request.json"),
            serde_json::json!({
                "workflow": "approval-flow",
                "task": "approve auth"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            sessions.join("workflow-state.json"),
            serde_json::json!({
                "workflow": "approval-flow",
                "task": "approve auth",
                "status": "waiting-approval",
                "steps_total": 1,
                "steps_completed": 0,
                "completed_steps": [],
                "skipped_steps": [],
                "retry_counts": {},
                "outputs": {},
                "budget_used": { "tokens": 0, "estimated_usd": 0.0 },
                "steps": [{
                    "id": "approve-me",
                    "agent": "Architect",
                    "status": "awaiting-approval",
                    "attempts": 1,
                    "output_key": "plan",
                    "output_preview": "draft plan",
                    "selected_route": null,
                    "selected_target": null,
                    "error": null
                }],
                "pending_approval": {
                    "step_id": "approve-me",
                    "agent": "Architect",
                    "output_key": "plan",
                    "content": "draft plan",
                    "content_preview": "draft plan"
                },
                "approval_decisions": {},
                "last_error": null
            })
            .to_string(),
        )
        .unwrap();

        let ts = test_server();
        let approval = ts
            .post("/api/runs/session-1/approval")
            .json(&serde_json::json!({
                "decision": "edit",
                "content": "approved plan v2"
            }))
            .await;
        approval.assert_status_ok();

        let response = ts.post("/api/runs/session-1/resume").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(
            body.get("status").and_then(|value| value.as_str()),
            Some("succeeded")
        );
        let resumed_id = body
            .get("session")
            .and_then(|value| value.as_str())
            .expect("resume response must include a new session id");
        assert_ne!(resumed_id, "session-1");
        assert_eq!(
            body.get("already_resumed")
                .and_then(|value| value.as_bool()),
            None
        );

        let resumed_again = ts.post("/api/runs/session-1/resume").await;
        resumed_again.assert_status_ok();
        let resumed_again_body: serde_json::Value = resumed_again.json();
        assert_eq!(
            resumed_again_body
                .get("session")
                .and_then(|value| value.as_str()),
            Some(resumed_id)
        );
        assert_eq!(
            resumed_again_body
                .get("already_resumed")
                .and_then(|value| value.as_bool()),
            Some(true)
        );

        let resumed_state: agent007_workflows::WorkflowRunState = serde_json::from_str(
            &std::fs::read_to_string(
                home.path()
                    .join("sessions")
                    .join(resumed_id)
                    .join("workflow-state.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            resumed_state.outputs.get("plan").map(String::as_str),
            Some("approved plan v2")
        );
        assert_eq!(
            resumed_state.status,
            agent007_workflows::WorkflowRunStatus::Succeeded
        );

        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn api_run_resume_rejects_external_workflow_run() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions").join("session-1");
        let workflows = home.path().join("workflows");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&workflows).unwrap();
        std::env::set_var("AGENT007_HOME", home.path());

        std::fs::write(
            workflows.join("approval-flow.toml"),
            r#"
name = "approval-flow"

[[steps]]
id = "approve-me"
agent = "Architect"
prompt = "Plan {{task}}"
output = "plan"
requires_approval = true
"#,
        )
        .unwrap();
        std::fs::write(
            sessions.join("meta.json"),
            serde_json::json!({
                "id": "session-1",
                "kind": "workflow",
                "task": "approve auth",
                "mode": "standalone",
                "provider": "mock",
                "started_at": chrono::Utc::now(),
                "finished_at": chrono::Utc::now(),
                "status": "awaiting-approval",
                "output_preview": "approval required"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            sessions.join("workflow-request.json"),
            serde_json::json!({
                "workflow": "approval-flow",
                "task": "approve auth"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            sessions.join("workflow-state.json"),
            serde_json::json!({
                "workflow": "approval-flow",
                "task": "approve auth",
                "status": "running",
                "steps_total": 1,
                "steps_completed": 1,
                "completed_steps": ["approve-me"],
                "skipped_steps": [],
                "retry_counts": {},
                "outputs": { "plan": "approved plan v2" },
                "budget_used": { "tokens": 0, "estimated_usd": 0.0 },
                "steps": [{
                    "id": "approve-me",
                    "agent": "Architect",
                    "status": "approved",
                    "attempts": 1,
                    "output_key": "plan",
                    "output_preview": "approved plan v2",
                    "selected_route": null,
                    "selected_target": null,
                    "error": null
                }],
                "pending_approval": null,
                "approval_decisions": {
                    "approve-me": {
                        "decision": "approve",
                        "content": null
                    }
                },
                "last_error": null
            })
            .to_string(),
        )
        .unwrap();

        let ts = test_server();
        let response = ts.post("/api/runs/session-1/resume").await;
        response.assert_status(StatusCode::CONFLICT);
        let body: serde_json::Value = response.json();
        assert_eq!(
            body.get("error").and_then(|value| value.as_str()),
            Some(EXTERNAL_WORKFLOW_CONTROL_ERROR)
        );

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn skill_import_source_parses_github_tree_url_as_package() {
        let parsed = super::parse_skill_import_source(
            "https://github.com/example/repo/tree/main/skills/review-skill",
        )
        .unwrap();

        match parsed {
            super::SkillImportSourceKind::GitHubDir {
                owner,
                repo,
                reference,
                path,
            } => {
                assert_eq!(owner, "example");
                assert_eq!(repo, "repo");
                assert_eq!(reference.as_deref(), Some("main"));
                assert_eq!(path, "skills/review-skill");
            }
            _ => panic!("expected GitHubDir import source"),
        }
    }

    #[test]
    fn normalize_skill_manifest_content_synthesizes_missing_frontmatter() {
        let (trigger, output) = super::normalize_skill_manifest_content(
            "Review this repo and suggest fixes.",
            "review-skill",
            "https://example.com/review-skill",
        )
        .unwrap();

        assert_eq!(trigger, "/review-skill");
        assert!(output.contains("trigger: /review-skill"));
        assert!(output.contains("name: Review Skill"));
    }

    #[test]
    fn relative_package_path_strips_package_root() {
        let relative = super::relative_package_path(
            "skills/review-skill/assets/example.txt",
            "skills/review-skill",
        )
        .unwrap();
        assert_eq!(relative, "assets/example.txt");
    }
}
