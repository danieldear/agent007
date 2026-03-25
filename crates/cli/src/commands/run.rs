use std::sync::{Arc, Mutex};
use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::config::Config;
use agent007_core::dispatcher::LocalDispatcher;
use agent007_core::orchestrator::OrchestratorAgent;
use agent007_core::task::Task;
use agent007_core::types::PromptStore;
use agent007_memory::store::MemoryStore;
use agent007_memory::vectordb::LanceDBStore;
use agent007_memory::Retriever;
use agent007_hooks::{HookConfig, HookExecutor};
use agent007_mcp::{McpClient, McpServerConfig};
use agent007_learning::{FeedbackCollector, LearningDispatcher, RewardScorer};
use agent007_learning::scorer::RewardWeights;
use agent007_learning::store::LearningStore;
use agent007_models::{MockProvider, ModelProvider, ModelRouter};
use agent007_skills::SkillExecutor;
use agent007_tui::{App, EventLoop};
use agent007_personas::PersonaRegistry;
use agent007_zones::{AuditLogger, ZoneChecker, ZoneConfig};
use agent007_core::tool_executor::ToolExecutor;

/// Return the agent007 home directory.
/// Checks AGENT007_HOME first, then falls back to $HOME/.agent007.
pub fn agent007_home() -> std::path::PathBuf {
    std::env::var("AGENT007_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(".agent007")
        })
}

pub struct Stack {
    pub dispatcher: Arc<LocalDispatcher>,
    pub memory_store: Arc<MemoryStore>,
    pub hook_executor: Arc<HookExecutor>,
    pub mcp_client: Arc<McpClient>,
    pub feedback_collector: Arc<FeedbackCollector>,
    pub learning_dispatcher: Arc<LearningDispatcher>,
    pub model_router: Arc<ModelRouter>,
    pub skill_executor: Arc<SkillExecutor>,
    pub persona_registry: Arc<PersonaRegistry>,
    pub orchestrator: Arc<OrchestratorAgent>,
    pub zone_checker: Arc<ZoneChecker>,
    pub audit_logger: Arc<AuditLogger>,
    pub tool_executor: Arc<ToolExecutor>,
    pub workflow_runner: Arc<agent007_workflows::WorkflowRunner>,
    pub cancel: CancellationToken,
    pub tracker: TaskTracker,
}

pub async fn build_stack(config: &Config) -> Result<Stack> {
    let cancel = CancellationToken::new();
    let tracker = TaskTracker::new();

    let home = agent007_home();

    // 1. Core dispatcher — returns Arc<LocalDispatcher>
    let dispatcher = LocalDispatcher::new(config.core.task_queue_capacity);

    // 2. Memory store
    let memory_dir = home.join("memory");
    let memory_store = Arc::new(MemoryStore::new(memory_dir));

    // 3. Hook executor — load from file or use defaults
    let hooks_path = home.join("hooks").join("hooks.toml");
    let hook_config = HookConfig::load(&hooks_path).unwrap_or_default();
    let hook_executor = Arc::new(HookExecutor::new(hook_config));

    // 4. MCP client (from config)
    let mcp_servers: Vec<McpServerConfig> = config
        .mcp
        .as_ref()
        .map(|m| {
            m.servers
                .iter()
                .map(|(name, cmd)| McpServerConfig {
                    name: name.clone(),
                    command: cmd.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mcp_client = Arc::new(McpClient::new(mcp_servers));

    // 5. Learning dispatcher — new() returns Self, wrap in Arc
    let learning_dispatcher = Arc::new(LearningDispatcher::new(512));

    // 6. Learning store — scoped() requires &Arc<MemoryStore>
    let learning_store = LearningStore::new(memory_store.scoped("learning"));

    // 7. Reward scorer
    let reward_weights = RewardWeights {
        completion: config.learning.reward_weights.completion,
        user_rating: config.learning.reward_weights.user_rating,
        tool_errors: config.learning.reward_weights.tool_errors,
        retries: config.learning.reward_weights.retries,
    };
    let scorer = RewardScorer::new(reward_weights);

    // 8. Feedback collector
    let feedback_collector = Arc::new(FeedbackCollector::new(
        dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        learning_store,
        scorer,
        learning_dispatcher.clone(),
    ));

    // 9. ModelRouter — use MockProvider if AGENT007_DRY_RUN=1, else real ClaudeProvider
    let is_dry_run = std::env::var("AGENT007_DRY_RUN").is_ok();
    let model_router = if is_dry_run {
        let mock = Arc::new(MockProvider::new("dry-run response", "mock"));
        let mut router = ModelRouter::new("mock");
        router.register("mock", mock as Arc<dyn ModelProvider>);
        router
    } else {
        let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
        let cfg_model = config.models.default.as_str();
        let model = if cfg_model.is_empty() || cfg_model == "mock" {
            "claude-sonnet-4-6"
        } else {
            cfg_model
        };
        if api_key.is_empty() {
            tracing::warn!("ANTHROPIC_API_KEY not set — skills will return placeholder responses");
            let mock = Arc::new(MockProvider::new("[set ANTHROPIC_API_KEY to enable real responses]", "mock"));
            let mut router = ModelRouter::new("mock");
            router.register("mock", mock as Arc<dyn ModelProvider>);
            router
        } else {
            let claude = Arc::new(agent007_models::ClaudeProvider::new(&api_key, model));
            let mut router = ModelRouter::new("claude");
            router.register("claude", claude as Arc<dyn ModelProvider>);
            router
        }
    };
    let model_router = Arc::new(model_router);

    // 10. SkillExecutor — needs Retriever (EmbeddingProvider + VectorDB) + ScopedMemoryStore
    let skills_dir = home.join("skills");
    let vectordb_path = home.join("vectordb");
    std::fs::create_dir_all(&vectordb_path)?;
    let vectordb_path_str = vectordb_path.to_string_lossy().to_string();

    // Use the model router as the embedding provider (MockProvider implements EmbeddingProvider)
    // For the vector DB, use LanceDB with a local path.
    // In dry-run, the VectorDB may fail — use a fallback no-op if unavailable.
    let skill_executor = build_skill_executor(
        model_router.clone() as Arc<dyn ModelProvider>,
        &vectordb_path_str,
        &memory_store,
        &skills_dir,
        is_dry_run,
    )
    .await?;
    let skill_executor = Arc::new(skill_executor);

    // 11. PersonaRegistry — load built-ins + user overrides from ~/.agent007/personas/
    let personas_dir = home.join("personas");
    let persona_registry = Arc::new(
        PersonaRegistry::load(&personas_dir).unwrap_or_else(|e| {
            tracing::warn!("failed to load persona overrides from {}: {}", personas_dir.display(), e);
            PersonaRegistry::built_in()
        })
    );

    // 12. Zones — load config from [zones] section and build ZoneChecker
    let zone_config = ZoneConfig {
        forbidden:    config.zones.forbidden.clone(),
        readonly:     config.zones.readonly.clone(),
        sensitive:    config.zones.sensitive.clone(),
        unrestricted: config.zones.unrestricted.clone(),
    };
    let zone_checker = Arc::new(
        ZoneChecker::new(&zone_config)
            .map_err(|e| anyhow::anyhow!("zones config error: {}", e))?
    );

    // Audit log at ~/.agent007/audit/audit.log
    let audit_dir = home.join("audit");
    let audit_log_path = audit_dir.join("audit.log");
    let audit_logger = Arc::new(AuditLogger::new(&audit_log_path));

    // ToolExecutor wired with zone checker + audit logger
    let tool_executor = Arc::new(
        ToolExecutor::new("OrchestratorAgent")
            .with_zone_checker(zone_checker.clone())
            .with_audit_logger(audit_logger.clone()),
    );

    // 13. OrchestratorAgent
    let prompt_store = Arc::new(Mutex::new(PromptStore::default()));
    let orchestrator = Arc::new(OrchestratorAgent::new(
        dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        model_router.clone(),
        prompt_store,
        cancel.clone(),
        config.core.max_agents,
    ));

    // 14. WorkflowRunner
    let workflows_dir = home.join("workflows");
    std::fs::create_dir_all(&workflows_dir)?;
    let workflow_runner = Arc::new(agent007_workflows::WorkflowRunner::new(
        persona_registry.clone() as Arc<dyn agent007_core::persona::PersonaProvider>,
        model_router.clone(),
        dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
    ));

    Ok(Stack {
        dispatcher,
        memory_store,
        hook_executor,
        mcp_client,
        feedback_collector,
        learning_dispatcher,
        model_router,
        skill_executor,
        persona_registry,
        orchestrator,
        zone_checker,
        audit_logger,
        tool_executor,
        workflow_runner,
        cancel,
        tracker,
    })
}

/// Build a SkillExecutor backed by LanceDB (or a no-op VectorDB in dry-run if LanceDB fails).
async fn build_skill_executor(
    provider: Arc<dyn ModelProvider>,
    vectordb_path: &str,
    memory_store: &Arc<MemoryStore>,
    _skills_dir: &std::path::Path,
    is_dry_run: bool,
) -> Result<SkillExecutor> {
    // Use MockProvider as the embedding provider (dim=384 as a reasonable default).
    let embedder = Arc::new(MockProvider::with_embedding_dim("", "mock-embed", 384))
        as Arc<dyn agent007_models::EmbeddingProvider>;

    // Try to build LanceDB; in dry-run, fall back to a no-op VectorDB if it fails.
    let db: Arc<dyn agent007_memory::VectorDB> = if is_dry_run {
        Arc::new(NoOpVectorDB)
    } else {
        let store = LanceDBStore::new(vectordb_path, "skills", 384).await
            .map_err(|e| anyhow::anyhow!("failed to open LanceDB at {}: {}", vectordb_path, e))?;
        Arc::new(store)
    };

    let retriever = Arc::new(Retriever::new(embedder, db, 5));
    let memory = memory_store.global();

    Ok(SkillExecutor::new(provider, retriever, memory))
}

/// A no-op VectorDB used in dry-run mode (no actual storage).
pub struct NoOpVectorDB;

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

pub async fn execute(config: Arc<Config>, task: String) -> Result<()> {
    let stack = build_stack(&config).await?;

    // Spawn feedback collector
    let collector = stack.feedback_collector.clone();
    stack.tracker.spawn(async move {
        if let Err(e) = collector.run().await {
            tracing::warn!("feedback collector error: {}", e);
        }
    });

    // Submit the task to the orchestrator
    let orchestrator = stack.orchestrator.clone();
    let task_desc = task.clone();
    stack.tracker.spawn(async move {
        let agent_task = Task::new(&task_desc);
        match orchestrator.run(agent_task).await {
            Ok(result) => {
                tracing::info!("task completed: {}", result.output);
            }
            Err(e) => {
                tracing::warn!("task failed: {}", e);
            }
        }
    });

    // When AGENT007_DRY_RUN=1, skip the TUI and just return Ok
    if std::env::var("AGENT007_DRY_RUN").is_ok() {
        stack.cancel.cancel();
        stack.tracker.close();
        // Do not await tracker.wait() in dry-run: the feedback-collector task holds a
        // broadcast stream that never ends, so waiting would block forever. The process
        // (or test) will clean up background tasks on drop.
        return Ok(());
    }

    // Construct App and EventLoop
    let mut app = App::default();
    app.push_log(format!("Starting task: {}", task));

    let event_loop = EventLoop::new(
        stack.dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        stack.learning_dispatcher.clone(),
    )
    .await?;

    event_loop.run(&mut app, stack.cancel.clone()).await?;
    stack.tracker.close();
    stack.tracker.wait().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    async fn run_command_builds_stack_without_panic() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Config::default();
        let stack = build_stack(&config).await.unwrap();
        // Verify stack was constructed (just check fields exist)
        assert!(stack.cancel.is_cancelled() == false);
        std::env::remove_var("AGENT007_DRY_RUN");
    }

    #[tokio::test]
    async fn build_stack_contains_persona_registry_with_builtins() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Config::default();
        let stack = build_stack(&config).await.unwrap();
        // PersonaRegistry must expose at least 10 built-in personas
        use agent007_core::PersonaProvider;
        assert!(stack.persona_registry.list().len() >= 10);
        std::env::remove_var("AGENT007_DRY_RUN");
    }

    #[tokio::test]
    async fn e2e_smoke_run_with_dry_run() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Arc::new(Config::default());
        let result = execute(config, "say hello".to_string()).await;
        std::env::remove_var("AGENT007_DRY_RUN");
        assert!(result.is_ok(), "run command failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn stack_contains_workflow_runner() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Config::default();
        let stack = build_stack(&config).await.unwrap();
        // WorkflowRunner is on the stack — test that it can validate an empty-step workflow
        use agent007_workflows::types::WorkflowDef;
        let def = WorkflowDef { name: "t".to_string(), description: None, steps: vec![], budget: None };
        let result = stack.workflow_runner.validate(&def);
        // Empty workflow validates to empty batches
        assert!(result.is_ok());
        std::env::remove_var("AGENT007_DRY_RUN");
    }

    #[tokio::test]
    async fn build_stack_creates_workflows_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path().to_str().unwrap());
        let config = Config::default();
        let _ = build_stack(&config).await.unwrap();
        assert!(tmp.path().join("workflows").exists());
        std::env::remove_var("AGENT007_HOME");
        std::env::remove_var("AGENT007_DRY_RUN");
    }
}
