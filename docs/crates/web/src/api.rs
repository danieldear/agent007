use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::server::AppState;

// ── request/response shapes ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RunRequest {
    pub task: String,
}

#[derive(Serialize)]
pub struct RunResponse {
    pub message: String,
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
    // Force dry-run so no TUI is spawned.
    std::env::set_var("AGENT007_DRY_RUN", "1");

    let cancel = state.cancel.clone();
    let prompt_store = Arc::new(std::sync::Mutex::new(
        agent007_core::types::PromptStore::default(),
    ));
    let orchestrator = Arc::new(agent007_core::orchestrator::OrchestratorAgent::new(
        state.dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        state.model_router.clone(),
        prompt_store,
        cancel,
        4,
    ));

    let core_task = agent007_core::Task::new(&payload.task);
    match orchestrator.run(core_task).await {
        Ok(_) => (
            StatusCode::OK,
            Json(RunResponse {
                message: "Task submitted to agent007 orchestrator.".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/skills` — list skills from `~/.agent007/skills/`.
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
    State(_state): State<AppState>,
    Json(payload): Json<SkillRunRequest>,
) -> impl IntoResponse {
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

    let mock_model =
        Arc::new(agent007_models::MockProvider::new("dry-run response", "mock"))
            as Arc<dyn agent007_models::ModelProvider>;

    let executor = agent007_skills::SkillExecutor::new(mock_model, retriever, memory);

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
        Ok(output) => (StatusCode::OK, Json(SkillRunResponse { output })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/status` — return current agent + task snapshot.
///
/// In this implementation the dispatcher does not expose an introspection API,
/// so we return empty lists with a placeholder avg_reward. A follow-on plan
/// can add `Dispatcher::snapshot()` to expose live data.
pub async fn status_handler(
    State(_state): State<AppState>,
) -> impl IntoResponse {
    Json(StatusResponse {
        agents: vec![],
        tasks: vec![],
        avg_reward: 0.0,
    })
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn agent007_home() -> std::path::PathBuf {
    std::env::var("AGENT007_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(".agent007")
        })
}

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
    }
    let fm: Fm = serde_yaml::from_str(parts[1]).ok()?;
    Some(serde_json::json!({
        "trigger": fm.trigger,
        "name": fm.name,
        "description": fm.description,
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
}
