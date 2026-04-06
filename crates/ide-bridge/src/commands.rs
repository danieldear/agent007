use std::sync::Arc;

use serde_json::Value;
use tower_lsp::Client;
use tower_lsp::lsp_types::{ExecuteCommandParams, MessageType};

use crate::error::IdeBridgeError;
use crate::server::BridgeConfig;

// ── public entry point ────────────────────────────────────────────────────────

pub async fn dispatch_command(
    client: &Client,
    config: &Arc<BridgeConfig>,
    params: ExecuteCommandParams,
) -> Result<Option<Value>, IdeBridgeError> {
    match params.command.as_str() {
        "agent007.run" => {
            let result = handle_run(config.clone(), params.arguments).await?;
            if let Some(ref val) = result {
                client
                    .show_message(MessageType::INFO, format!("agent007: {val}"))
                    .await;
            }
            Ok(result)
        }
        "agent007.skillList" => handle_skill_list(config.clone()).await,
        "agent007.skillRun" => handle_skill_run(config.clone(), params.arguments).await,
        other => Err(IdeBridgeError::UnknownCommand(other.to_string())),
    }
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// `agent007.run` — `{ "task": string }` → submits to orchestrator, returns
/// `"Task submitted to agent007 orchestrator."` as a JSON string.
pub(crate) async fn handle_run(
    config: Arc<BridgeConfig>,
    arguments: Vec<Value>,
) -> Result<Option<Value>, IdeBridgeError> {
    let arg = arguments
        .into_iter()
        .next()
        .ok_or_else(|| IdeBridgeError::MissingArgument("task".to_string()))?;

    let task = arg
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IdeBridgeError::MissingArgument("task".to_string()))?
        .to_string();

    // Build a minimal stack using agent007-core types directly.
    let cancel = tokio_util::sync::CancellationToken::new();
    let dispatcher = agent007_core::dispatcher::LocalDispatcher::new(64);

    // Minimal model router backed by MockProvider.
    let mock = Arc::new(agent007_models::MockProvider::new("dry-run response", "mock"))
        as Arc<dyn agent007_models::ModelProvider>;
    let mut router = agent007_models::ModelRouter::new("mock");
    router.register("mock", mock);
    let model_router = Arc::new(router);

    let prompt_store = Arc::new(std::sync::Mutex::new(
        agent007_core::types::PromptStore::default(),
    ));
    let orchestrator = agent007_core::orchestrator::OrchestratorAgent::new(
        dispatcher as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        model_router,
        prompt_store,
        cancel,
        config.max_agents.max(1),
    );

    let core_task = agent007_core::Task::new(&task);
    orchestrator
        .run(core_task)
        .await
        .map_err(IdeBridgeError::Orchestrator)?;

    Ok(Some(Value::String(
        "Task submitted to agent007 orchestrator.".to_string(),
    )))
}

/// `agent007.skillList` — returns a JSON array of `{ trigger, name, description }`.
pub(crate) async fn handle_skill_list(
    _config: Arc<BridgeConfig>,
) -> Result<Option<Value>, IdeBridgeError> {
    let skills_dir = agent007_home().join("skills");

    // If the directory does not exist yet, return an empty array gracefully.
    if !skills_dir.exists() {
        return Ok(Some(Value::Array(vec![])));
    }

    let mut entries = tokio::fs::read_dir(&skills_dir)
        .await
        .map_err(IdeBridgeError::Io)?;

    let mut skills: Vec<Value> = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(IdeBridgeError::Io)? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(IdeBridgeError::Io)?;
        if let Some(fm) = parse_frontmatter(&content) {
            skills.push(fm);
        }
    }

    Ok(Some(Value::Array(skills)))
}

/// `agent007.skillRun` — `{ "trigger": string, "args": string }` → runs skill,
/// returns output as a JSON string.
pub(crate) async fn handle_skill_run(
    _config: Arc<BridgeConfig>,
    arguments: Vec<Value>,
) -> Result<Option<Value>, IdeBridgeError> {
    let arg = arguments
        .into_iter()
        .next()
        .ok_or_else(|| IdeBridgeError::MissingArgument("trigger".to_string()))?;

    let trigger = arg
        .get("trigger")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IdeBridgeError::MissingArgument("trigger".to_string()))?
        .to_string();

    let args = arg
        .get("args")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Build a minimal SkillExecutor (dry-run, no real VectorDB).
    let embedder = Arc::new(agent007_models::MockProvider::with_embedding_dim(
        "",
        "mock-embed",
        384,
    )) as Arc<dyn agent007_models::EmbeddingProvider>;

    let db = Arc::new(NoOpVectorDB) as Arc<dyn agent007_memory::VectorDB>;
    let retriever = Arc::new(agent007_memory::Retriever::new(embedder, db, 5));

    let tmp = tempfile::TempDir::new().map_err(IdeBridgeError::Io)?;
    let memory_store = Arc::new(agent007_memory::store::MemoryStore::new(tmp.path()));
    let memory = memory_store.global();
    let global_home = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".agent007")
        .join("memory");
    let global_store = Arc::new(agent007_memory::store::MemoryStore::new(global_home));
    let global_memory = global_store.scoped("global");

    let mock_model = Arc::new(agent007_models::MockProvider::new("dry-run response", "mock"))
        as Arc<dyn agent007_models::ModelProvider>;

    let executor = agent007_skills::SkillExecutor::new(mock_model, retriever, memory)
        .with_global_memory(global_memory);

    let skills_dir = agent007_home().join("skills");
    let loader = agent007_skills::SkillLoader::new(&skills_dir);
    let skills = loader
        .load_all()
        .map_err(|e| IdeBridgeError::Transport(e.to_string()))?;

    let skill = skills
        .into_iter()
        .find(|s| s.trigger() == trigger)
        .ok_or_else(|| IdeBridgeError::UnknownCommand(trigger.clone()))?;

    let output = executor
        .execute(&skill, &args)
        .await
        .map_err(|e| IdeBridgeError::Transport(e.to_string()))?;

    Ok(Some(Value::String(output)))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn agent007_home() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("AGENT007_HOME") {
        return std::path::PathBuf::from(p);
    }
    let mut dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    loop {
        let candidate = dir.join(".agent007");
        if candidate.is_dir() {
            return candidate;
        }
        if !dir.pop() {
            break;
        }
    }
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".agent007")
}

/// Parse YAML frontmatter from a skill `.md` file into a JSON object with
/// keys `trigger`, `name`, `description`.
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

/// No-op VectorDB used in dry-run mode.
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
    use super::*;
    use tower_lsp::lsp_types::ExecuteCommandParams;

    fn make_params(cmd: &str, args: Vec<serde_json::Value>) -> ExecuteCommandParams {
        ExecuteCommandParams {
            command: cmd.to_string(),
            arguments: args,
            work_done_progress_params: Default::default(),
        }
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    async fn run_command_dry_run_succeeds() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let cfg = Arc::new(crate::server::BridgeConfig::default());
        let params = make_params(
            "agent007.run",
            vec![serde_json::json!({"task": "say hello"})],
        );
        let result = handle_run(cfg, params.arguments).await;
        std::env::remove_var("AGENT007_DRY_RUN");
        assert!(result.is_ok());
        let val = result.unwrap();
        assert!(val.is_some());
    }

    #[tokio::test]
    async fn skill_list_returns_array() {
        let cfg = Arc::new(crate::server::BridgeConfig::default());
        let result = handle_skill_list(cfg).await;
        assert!(result.is_ok());
        let val = result.unwrap().unwrap();
        assert!(val.is_array());
    }

    #[test]
    fn unknown_command_error_variant() {
        let err = IdeBridgeError::UnknownCommand("agent007.bogus".to_string());
        assert!(matches!(err, IdeBridgeError::UnknownCommand(_)));
    }
}
