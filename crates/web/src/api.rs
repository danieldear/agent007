use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use agent007_core::paths::agent007_home;
use agent007_workflows::{
    WorkflowError, WorkflowLoader, WorkflowRunRequest, WorkflowRunState, WorkflowSourceRef,
};

use crate::server::AppState;

// ── request/response shapes ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RunRequest {
    pub task: String,
}

#[derive(Serialize)]
pub struct RunResponse {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Deserialize)]
pub struct ApprovalRequest {
    pub step: Option<String>,
    pub decision: String,
    pub content: Option<String>,
}

#[derive(Deserialize)]
pub struct SkillRunRequest {
    pub trigger: String,
    #[serde(default)]
    pub args: String,
}

#[derive(Serialize)]
pub struct SkillRunResponse {
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub agents: Vec<Value>,
    pub tasks: Vec<Value>,
    pub avg_reward: f64,
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
    let run = match store.create_run("web-run", &payload.task, "standalone", None) {
        Ok(run) => run,
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
        Ok(_) => {
            let _ = store.finish_run(
                &run.id,
                true,
                "Task submitted to agent007 orchestrator.",
            );
            (
                StatusCode::OK,
                Json(RunResponse {
                    message: "Task submitted to agent007 orchestrator.".to_string(),
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

/// `GET /api/skills` — list skills from `.agent007/skills/` (project-local or global home).
pub async fn skills_handler(
    State(_state): State<AppState>,
) -> impl IntoResponse {
    let skills_dir = agent007_home().join("skills");

    if !skills_dir.exists() {
        return Json(serde_json::json!([])).into_response();
    }

    let read = std::fs::read_dir(&skills_dir);
    let Ok(entries) = read else {
        return Json(serde_json::json!([])).into_response();
    };

    let mut skills: Vec<Value> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(fm) = parse_frontmatter(&content) {
                skills.push(fm);
            }
        }
    }

    Json(Value::Array(skills)).into_response()
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

    let tmp = match tempfile::TempDir::new() {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let memory_store = Arc::new(agent007_memory::store::MemoryStore::new(tmp.path()));
    let memory = memory_store.global();

    let model =
        state.model_router.clone() as Arc<dyn agent007_models::ModelProvider>;

    let executor = agent007_skills::SkillExecutor::new(model, retriever, memory);
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

    let skill = match skills.into_iter().find(|s| s.trigger() == payload.trigger) {
        Some(s) => s,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("skill not found: {}", payload.trigger) })),
            )
                .into_response()
        }
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
pub async fn status_handler(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let m = crate::metrics::snapshot_with_shared_state(
        state.metrics.lock().await.clone(),
        agent007_home(),
    );
    let tasks = m.recent_tasks.iter().map(|task| {
        serde_json::json!({
            "id": task.id,
            "task": task.task,
            "status": task.status,
            "agent": task.agent,
            "tokens": task.tokens,
            "started_at": task.started_at,
            "finished_at": task.finished_at,
        })
    }).collect();
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
pub async fn stats_handler(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let mut m = crate::metrics::snapshot_with_shared_state(
        state.metrics.lock().await.clone(),
        agent007_home(),
    );

    let home = agent007_home();
    let skills_count = count_dir_files(&home.join("skills"), "md");
    let workflows_count = count_dir_files(&home.join("workflows"), "yaml")
        + count_dir_files(&home.join("workflows"), "yml");
    let personas_count = count_dir_files(&home.join("personas"), "toml");
    let memory_keys = count_dir_files(&home.join("memory"), "md");
    m.update_inventory(skills_count, workflows_count, personas_count, memory_keys);

    let snapshot = m.clone();
    drop(m);
    Json(snapshot).into_response()
}

pub async fn runs_handler(
    State(_state): State<AppState>,
) -> impl IntoResponse {
    let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
    match store.list_runs(25) {
        Ok(runs) => Json(serde_json::to_value(runs).unwrap_or_else(|_| serde_json::json!([]))).into_response(),
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
    let mut state: agent007_workflows::WorkflowRunState = match store.read_json_artifact(&id, "workflow-state.json") {
        Ok(state) => state,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let step_id = payload
        .step
        .clone()
        .or_else(|| state.pending_approval.as_ref().map(|pending| pending.step_id.clone()));
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

    let store = Arc::new(agent007_core::RunStore::new(agent007_home().join("sessions")));
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
    let workflow_ref = match store
        .read_json_artifact_optional::<WorkflowSourceRef>(&id, "workflow-source.json")
    {
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

    let workflow_state: WorkflowRunState = match store.read_json_artifact(&id, "workflow-state.json") {
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

    let loader = WorkflowLoader::new(agent007_home().join("workflows"));
    let def = match loader.load_named(&workflow_ref) {
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
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some(ext))
                .count() as u32
        })
        .unwrap_or(0)
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

    Ok(ApprovalDecision { decision: kind, content })
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

pub async fn personas_list_handler(
    State(_state): State<AppState>,
) -> impl IntoResponse {
    let personas_dir = agent007_home().join("personas");
    let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
        .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
    use agent007_core::PersonaProvider;
    let personas: Vec<Value> = registry.list().iter().map(|p| {
        serde_json::json!({
            "name": p.name,
            "description": p.description,
            "preferred_model": p.preferred_model,
            "allowed_tools": p.allowed_tools,
            "system_prompt": p.system_prompt,
        })
    }).collect();
    Json(Value::Array(personas)).into_response()
}

pub async fn persona_save_handler(
    State(_state): State<AppState>,
    Json(payload): Json<PersonaSaveRequest>,
) -> impl IntoResponse {
    let personas_dir = agent007_home().join("personas");
    if let Err(e) = std::fs::create_dir_all(&personas_dir) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response();
    }

    let filename = payload.name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>().to_lowercase();
    let path = personas_dir.join(format!("{filename}.toml"));

    let tools = payload.allowed_tools.unwrap_or_default();
    let tools_str = tools.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", ");
    let model = payload.preferred_model.as_deref().unwrap_or("codex");
    let prompt = payload.system_prompt.as_deref().unwrap_or("");

    let content = format!(
        "name            = \"{}\"\n\
         description     = \"{}\"\n\
         preferred_model = \"{}\"\n\
         allowed_tools   = [{}]\n\n\
         system_prompt   = \"\"\"\n{}\n\"\"\"\n",
        payload.name, payload.description.replace('"', "\\\""), model, tools_str, prompt,
    );

    match std::fs::write(&path, &content) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "path": path.display().to_string() })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn persona_delete_handler(
    State(_state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let personas_dir = agent007_home().join("personas");
    let filename = name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>().to_lowercase();
    let path = personas_dir.join(format!("{filename}.toml"));

    if path.exists() {
        match std::fs::remove_file(&path) {
            Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        }
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "not found" }))).into_response()
    }
}

// ── Workflow CRUD ──────────────────────────────────────────────────────────────

pub async fn workflows_list_handler(
    State(_state): State<AppState>,
) -> impl IntoResponse {
    let wf_dir = agent007_home().join("workflows");
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&wf_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            if ext == Some("yaml") || ext == Some("yml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    Json(names).into_response()
}

pub async fn workflow_get_handler(
    State(_state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let wf_dir = agent007_home().join("workflows");
    let yaml_path = wf_dir.join(format!("{name}.yaml"));
    let yml_path = wf_dir.join(format!("{name}.yml"));
    let path = if yaml_path.exists() { yaml_path } else if yml_path.exists() { yml_path } else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "not found" }))).into_response();
    };

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            match serde_yaml::from_str::<Value>(&content) {
                Ok(val) => Json(val).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn workflow_save_handler(
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("untitled");
    let wf_dir = agent007_home().join("workflows");
    if let Err(e) = std::fs::create_dir_all(&wf_dir) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response();
    }

    let path = wf_dir.join(format!("{name}.yaml"));
    match serde_yaml::to_string(&payload) {
        Ok(yaml) => {
            match std::fs::write(&path, &yaml) {
                Ok(()) => Json(serde_json::json!({ "ok": true, "path": path.display().to_string() })).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

// ── Skill save ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SkillSaveRequest {
    pub name: String,
    pub trigger: String,
    pub description: String,
    pub model: Option<String>,
    pub template: String,
}

pub async fn skill_save_handler(
    State(_state): State<AppState>,
    Json(payload): Json<SkillSaveRequest>,
) -> impl IntoResponse {
    let skills_dir = agent007_home().join("skills");
    if let Err(e) = std::fs::create_dir_all(&skills_dir) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response();
    }

    let filename = payload.name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>();
    let path = skills_dir.join(format!("{filename}.md"));
    let model = payload.model.as_deref().unwrap_or("codex");

    let content = format!(
        "---\nname: {}\ntrigger: {}\ndescription: {}\nmodel: {}\n---\n{}\n",
        payload.name, payload.trigger, payload.description, model, payload.template,
    );

    match std::fs::write(&path, &content) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "path": path.display().to_string() })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

// ── Memory list ───────────────────────────────────────────────────────────────

pub async fn memory_list_handler(
    State(_state): State<AppState>,
    Path(scope): Path<String>,
) -> impl IntoResponse {
    let home = agent007_home();
    let scope_dir = if scope == "global" {
        home.join("memory")
    } else {
        home.join("memory").join(&scope)
    };

    let mut keys = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&scope_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    keys.push(stem.to_string());
                }
            }
        }
    }
    keys.sort();
    Json(keys).into_response()
}

// ── Workflow Templates ────────────────────────────────────────────────────────

pub async fn workflow_templates_list_handler() -> impl IntoResponse {
    Json(get_workflow_templates()).into_response()
}

pub async fn workflow_template_get_handler(
    Path(name): Path<String>,
) -> impl IntoResponse {
    let templates = get_workflow_templates();
    match templates.iter().find(|t| t.get("name").and_then(|v| v.as_str()) == Some(name.as_str())) {
        Some(t) => Json(t.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "template not found" }))).into_response(),
    }
}

// ── Skill Registry & Import ──────────────────────────────────────────────────

pub async fn skill_get_handler(
    State(_state): State<AppState>,
    Path(trigger): Path<String>,
) -> impl IntoResponse {
    let skills_dir = agent007_home().join("skills");
    let Ok(entries) = std::fs::read_dir(&skills_dir) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "skills dir not found" }))).into_response();
    };

    let target_trigger = format!("/{trigger}");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(fm) = parse_frontmatter(&content) {
                if fm.get("trigger").and_then(|v| v.as_str()) == Some(&target_trigger) {
                    let mut result = fm;
                    if let Some(obj) = result.as_object_mut() {
                        let parts: Vec<&str> = content.splitn(3, "---").collect();
                        if parts.len() >= 3 {
                            obj.insert("template".to_string(), serde_json::Value::String(parts[2].trim().to_string()));
                        }
                    }
                    return Json(result).into_response();
                }
            }
        }
    }

    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "skill not found" }))).into_response()
}

#[derive(Deserialize)]
pub struct SkillImportRequest {
    pub url: String,
}

pub async fn skill_import_handler(
    State(_state): State<AppState>,
    Json(payload): Json<SkillImportRequest>,
) -> impl IntoResponse {
    let url = normalize_github_url(&payload.url);

    let client = reqwest::Client::new();
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    };

    if !resp.status().is_success() {
        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": format!("HTTP {}", resp.status()) }))).into_response();
    }

    let content = match resp.text().await {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    };

    if content.len() > 100_000 {
        return (StatusCode::PAYLOAD_TOO_LARGE, Json(serde_json::json!({ "error": "skill file exceeds 100KB limit" }))).into_response();
    }

    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid skill: missing frontmatter" }))).into_response();
    }

    #[derive(serde::Deserialize)]
    struct MinFm { trigger: String }
    let fm: MinFm = match serde_yaml::from_str(parts[1]) {
        Ok(f) => f,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": format!("invalid frontmatter: {e}") }))).into_response(),
    };

    let filename = fm.trigger.trim_start_matches('/')
        .chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>();
    let skills_dir = agent007_home().join("skills");
    let _ = std::fs::create_dir_all(&skills_dir);
    let path = skills_dir.join(format!("{filename}.md"));

    match std::fs::write(&path, &content) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "trigger": fm.trigger, "path": path.display().to_string() })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn skill_registry_handler() -> impl IntoResponse {
    let registry_url = "https://raw.githubusercontent.com/agent007-community/skills/main/registry.json";
    let client = reqwest::Client::new();
    match client.get(registry_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(val) => Json(val).into_response(),
                Err(_) => Json(serde_json::json!([])).into_response(),
            }
        }
        _ => Json(serde_json::json!([])).into_response(),
    }
}

fn normalize_github_url(url: &str) -> String {
    let url = url.trim();
    if url.contains("github.com") && url.contains("/blob/") {
        url.replace("github.com", "raw.githubusercontent.com").replace("/blob/", "/")
    } else {
        url.to_string()
    }
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
    ]
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn parse_frontmatter(content: &str) -> Option<Value> {
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct Fm {
        name: String,
        description: String,
        trigger: String,
        #[serde(default)]
        category: Option<String>,
    }
    let fm: Fm = serde_yaml::from_str(parts[1]).ok()?;
    Some(serde_json::json!({
        "trigger": fm.trigger,
        "name": fm.name,
        "description": fm.description,
        "category": fm.category.unwrap_or_else(|| "custom".to_string()),
    }))
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
    use axum_test::TestServer;
    use crate::server::WebServer;

    fn test_server() -> TestServer {
        let server = WebServer::new_test();
        TestServer::new(server.into_router()).unwrap()
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    async fn api_run_accepts_task() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let ts = test_server();
        let response = ts
            .post("/api/run")
            .json(&serde_json::json!({ "task": "hello" }))
            .await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body.get("message").is_some());
        assert!(body.get("session").is_some());
        std::env::remove_var("AGENT007_DRY_RUN");
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
        assert_eq!(arr.len(), 5);
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
            .to_string() + "\n",
        )
        .unwrap();

        let ts = test_server();
        let response = ts.get("/api/stats").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body.get("completed_tasks").and_then(|value| value.as_u64()), Some(1));
        assert_eq!(body.get("session_requests").and_then(|value| value.as_u64()), Some(1));
        assert_eq!(body.get("total_tokens").and_then(|value| value.as_u64()), Some(222));

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
        assert_eq!(body.get("status").and_then(|value| value.as_str()), Some("succeeded"));
        let resumed_id = body
            .get("session")
            .and_then(|value| value.as_str())
            .expect("resume response must include a new session id");
        assert_ne!(resumed_id, "session-1");

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
}
