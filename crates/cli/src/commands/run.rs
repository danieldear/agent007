use anyhow::Result;
use chrono::Utc;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::config::Config;
use agent007_core::dispatcher::LocalDispatcher;
use agent007_core::events::AgentEvent;
use agent007_core::orchestrator::OrchestratorAgent;
use agent007_core::run_store::RunStore;
use agent007_core::task::Task;
use agent007_core::tool_executor::ToolExecutor;
use agent007_core::types::PromptStore;
use agent007_hooks::{HookConfig, HookExecutor};
use agent007_learning::scorer::RewardWeights;
use agent007_learning::store::LearningStore;
use agent007_learning::{FeedbackCollector, LearningDispatcher, RewardScorer};
use agent007_mcp::{McpClient, McpServerConfig};
use agent007_memory::store::{MemoryEntryType, MemoryMeta, MemoryStore};
use agent007_memory::vectordb::LanceDBStore;
use agent007_memory::Indexer;
use agent007_memory::Retriever;
use agent007_models::{
    ClaudeProvider, CodexProvider, EmbeddingProvider, MockProvider, ModelProvider, ModelRouter,
    OllamaEmbeddingProvider, OllamaProvider,
};
use agent007_personas::PersonaRegistry;
use agent007_skills::SkillExecutor;
use agent007_tui::{App, EventLoop};
use agent007_zones::{AuditLogger, ZoneChecker, ZoneConfig};

pub use agent007_core::paths::{
    agent007_global_home, agent007_home, agent007_project_home, agent007_write_home,
};

pub struct Stack {
    pub dispatcher: Arc<LocalDispatcher>,
    pub run_store: Arc<RunStore>,
    pub memory_store: Arc<MemoryStore>,
    pub hook_executor: Arc<HookExecutor>,
    pub mcp_client: Arc<AsyncMutex<McpClient>>,
    pub feedback_collector: Arc<FeedbackCollector>,
    pub learning_dispatcher: Arc<LearningDispatcher>,
    pub model_router: Arc<ModelRouter>,
    pub skill_executor: Arc<SkillExecutor>,
    pub rag_warmup_indexed_docs: usize,
    pub persona_registry: Arc<PersonaRegistry>,
    pub orchestrator: Arc<OrchestratorAgent>,
    pub zone_checker: Arc<ZoneChecker>,
    pub audit_logger: Arc<AuditLogger>,
    pub tool_executor: Arc<ToolExecutor>,
    pub workflow_runner: Arc<agent007_workflows::WorkflowRunner>,
    pub cancel: CancellationToken,
    pub tracker: TaskTracker,
}

fn configured_persona_registry() -> PersonaRegistry {
    let mut dirs = Vec::new();
    if let Some(project_home) = agent007_project_home() {
        dirs.push(project_home.join("personas"));
    }
    let global_dir = agent007_global_home().join("personas");
    if !dirs.iter().any(|dir| dir == &global_dir) {
        dirs.push(global_dir);
    }
    PersonaRegistry::load_from_dirs(dirs.iter().map(|dir| dir.as_path()))
        .unwrap_or_else(|_| PersonaRegistry::built_in())
}

struct ResilientEmbeddingProvider {
    primary: Arc<dyn EmbeddingProvider>,
    primary_name: String,
    fallback_dim: usize,
    warned: AtomicBool,
}

impl ResilientEmbeddingProvider {
    fn new(primary: Arc<dyn EmbeddingProvider>, fallback_dim: usize) -> Self {
        Self {
            primary_name: primary.name().to_string(),
            primary,
            fallback_dim,
            warned: AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for ResilientEmbeddingProvider {
    fn name(&self) -> &str {
        self.primary_name.as_str()
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, agent007_models::ModelError> {
        match self.primary.embed(text).await {
            Ok(embedding) => Ok(embedding),
            Err(err) => {
                if !self.warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        provider = %self.primary_name,
                        fallback_dim = self.fallback_dim,
                        error = %err,
                        "embedding provider unavailable; falling back to zero-vector retrieval"
                    );
                }
                Ok(vec![0.0; self.fallback_dim])
            }
        }
    }
}

pub fn is_dry_run() -> bool {
    std::env::var("AGENT007_DRY_RUN").is_ok()
}

pub fn has_anthropic_api_key() -> bool {
    std::env::var("ANTHROPIC_API_KEY")
        .map(|k| !k.is_empty())
        .unwrap_or(false)
}

pub fn has_openai_api_key() -> bool {
    std::env::var("OPENAI_API_KEY")
        .map(|k| !k.is_empty())
        .unwrap_or(false)
}

pub fn standalone_mode_available(config: &Config) -> bool {
    is_dry_run()
        || has_anthropic_api_key()
        || has_openai_api_key()
        || config.models.ollama.is_some()
}

pub fn runtime_mode_label(config: &Config) -> &'static str {
    if is_dry_run() {
        "dry-run"
    } else if selected_runtime_provider(config).as_deref() == Some("ollama") {
        "local-ollama"
    } else if standalone_mode_available(config) {
        "standalone"
    } else {
        "hosted-mcp"
    }
}

pub fn available_runtime_providers(config: &Config) -> Vec<String> {
    if is_dry_run() {
        return vec!["mock".to_string()];
    }

    let mut providers = Vec::new();
    if has_anthropic_api_key() {
        providers.push("claude".to_string());
    }
    if has_openai_api_key() {
        providers.push("codex".to_string());
    }
    if config.models.ollama.is_some() {
        providers.push("ollama".to_string());
    }
    providers
}

pub fn selected_runtime_provider(config: &Config) -> Option<String> {
    let available = available_runtime_providers(config);
    if available.is_empty() {
        return None;
    }

    let requested = config.models.default_provider();
    if available.iter().any(|provider| provider == &requested) {
        Some(requested)
    } else {
        Some(available[0].clone())
    }
}

pub fn selected_runtime_model(config: &Config) -> Option<String> {
    selected_runtime_provider(config)
        .map(|provider| config.models.default_model_for_provider(&provider))
}

fn should_use_non_interactive_mode() -> bool {
    is_dry_run()
        || std::env::var("AGENT007_NO_TUI")
            .map(|value| value != "0")
            .unwrap_or(false)
        || !std::io::stderr().is_terminal()
}

pub fn build_model_router(config: &Config, is_dry_run: bool) -> ModelRouter {
    if is_dry_run {
        let mock = Arc::new(MockProvider::new("dry-run response", "mock"));
        let mut router = ModelRouter::new("mock");
        router.register("mock", mock as Arc<dyn ModelProvider>);
        return router;
    }

    let mut router = ModelRouter::new("mock");
    let mut available = Vec::new();

    if has_anthropic_api_key() {
        let model = config.models.default_model_for_provider("claude");
        let claude = Arc::new(ClaudeProvider::new(
            &std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            &model,
        ));
        router.register("claude", claude as Arc<dyn ModelProvider>);
        router.alias(&model, "claude");
        available.push("claude".to_string());
    }

    if has_openai_api_key() {
        let model = config.models.default_model_for_provider("codex");
        let codex = Arc::new(CodexProvider::new(
            &std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            &model,
        ));
        router.register("codex", codex as Arc<dyn ModelProvider>);
        router.alias(&model, "codex");
        available.push("codex".to_string());
    }

    if let Some(ollama) = &config.models.ollama {
        let model = ollama.default_model.clone();
        let provider = Arc::new(OllamaProvider::new(&ollama.base_url, &model));
        router.register("ollama", provider.clone() as Arc<dyn ModelProvider>);
        router.alias(&model, "ollama");
        router.alias(&format!("ollama/{model}"), "ollama");
        available.push("ollama".to_string());
    }

    if available.is_empty() {
        tracing::warn!(
            "no real model providers configured — falling back to mock responses (set ANTHROPIC_API_KEY, OPENAI_API_KEY, or [models.ollama])"
        );
        let mock = Arc::new(MockProvider::new(
            "[configure ANTHROPIC_API_KEY, OPENAI_API_KEY, or [models.ollama] to enable real responses]",
            "mock",
        ));
        router.register("mock", mock as Arc<dyn ModelProvider>);
        return router;
    }

    let requested_default = config.models.default_provider();
    let default_provider = if available
        .iter()
        .any(|provider| provider == &requested_default)
    {
        requested_default
    } else {
        let fallback = available[0].clone();
        tracing::warn!(
            requested = %config.models.default,
            fallback = %fallback,
            "configured default provider is unavailable; falling back to first available provider"
        );
        fallback
    };
    router = ModelRouter::new(&default_provider);

    if has_anthropic_api_key() {
        let model = config.models.default_model_for_provider("claude");
        let claude = Arc::new(ClaudeProvider::new(
            &std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            &model,
        ));
        router.register("claude", claude as Arc<dyn ModelProvider>);
        router.alias(&model, "claude");
    }

    if has_openai_api_key() {
        let model = config.models.default_model_for_provider("codex");
        let codex = Arc::new(CodexProvider::new(
            &std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            &model,
        ));
        router.register("codex", codex as Arc<dyn ModelProvider>);
        router.alias(&model, "codex");
    }

    if let Some(ollama) = &config.models.ollama {
        let model = ollama.default_model.clone();
        let provider = Arc::new(OllamaProvider::new(&ollama.base_url, &model));
        router.register("ollama", provider.clone() as Arc<dyn ModelProvider>);
        router.alias(&model, "ollama");
        router.alias(&format!("ollama/{model}"), "ollama");
    }

    if let Some(routing) = &config.models.routing {
        for (task_type, provider) in [
            ("code_completion", routing.code_completion.as_deref()),
            ("reasoning", routing.reasoning.as_deref()),
            ("fast_local", routing.fast_local.as_deref()),
            ("sensitive", routing.sensitive.as_deref()),
            ("default", routing.default.as_deref()),
        ] {
            if let Some(provider) = provider {
                let (provider_name, model_name) =
                    config.models.resolve_provider_and_model(Some(provider));
                router.add_rule(task_type, &provider_name);
                if model_name != provider_name {
                    router.alias(&model_name, &provider_name);
                }
            }
        }
    } else {
        // Smart defaults: route code tasks to codex if available, reasoning/sensitive to claude,
        // fast local tasks to ollama.
        if available.contains(&"codex".to_string()) {
            router.add_rule("code_completion", "codex");
        }
        if available.contains(&"claude".to_string()) {
            router.add_rule("reasoning", "claude");
            router.add_rule("sensitive", "claude");
        }
        if available.contains(&"ollama".to_string()) {
            router.add_rule("fast_local", "ollama");
        }
    }

    let (default_provider_name, default_model_name) = config
        .models
        .resolve_provider_and_model(Some(&config.models.default));
    if default_model_name != default_provider_name {
        router.alias(&default_model_name, &default_provider_name);
    }

    router
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

    // 2b. Durable run/session store
    let sessions_dir = home.join("sessions");
    std::fs::create_dir_all(&sessions_dir)?;
    let run_store = Arc::new(RunStore::new(sessions_dir));

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
                .map(|(name, entry)| entry.to_server_config(name))
                .collect()
        })
        .unwrap_or_default();
    let mut mcp_client = McpClient::new(mcp_servers);
    if let Err(error) = mcp_client.connect().await {
        tracing::warn!("failed to connect configured MCP servers: {error}");
    }
    let mcp_client = Arc::new(AsyncMutex::new(mcp_client));

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
    let is_dry_run = is_dry_run();
    let model_router = build_model_router(config, is_dry_run);
    let model_router = Arc::new(model_router);

    // 10. SkillExecutor — needs Retriever (EmbeddingProvider + VectorDB) + ScopedMemoryStore
    let skills_dir = home.join("skills");
    let vectordb_path = home.join("vectordb");
    std::fs::create_dir_all(&vectordb_path)?;
    let vectordb_path_str = vectordb_path.to_string_lossy().to_string();

    // Use the model router as the embedding provider (MockProvider implements EmbeddingProvider)
    // For the vector DB, use LanceDB with a local path.
    // In dry-run, the VectorDB may fail — use a fallback no-op if unavailable.
    let lsp_categories = config
        .lsp
        .as_ref()
        .map(|l| l.inject_for_categories.clone())
        .unwrap_or_else(|| vec!["code_completion".to_string(), "reasoning".to_string()]);
    let (skill_executor, rag_warmup_indexed_docs) = build_skill_executor(
        model_router.clone() as Arc<dyn ModelProvider>,
        config,
        &vectordb_path_str,
        &memory_store,
        &skills_dir,
        is_dry_run,
    )
    .await?;
    let skill_executor = skill_executor
        .with_router(model_router.clone())
        .with_lsp_categories(lsp_categories);
    let skill_executor = Arc::new(skill_executor);

    // 11. PersonaRegistry — load built-ins + user overrides from ~/.agent007/personas/
    let persona_registry = Arc::new(configured_persona_registry());

    // 12. Zones — load config from [zones] section and build ZoneChecker
    let zone_config = ZoneConfig {
        forbidden: config.zones.forbidden.clone(),
        readonly: config.zones.readonly.clone(),
        sensitive: config.zones.sensitive.clone(),
        unrestricted: config.zones.unrestricted.clone(),
    };
    let zone_checker = Arc::new(
        ZoneChecker::new(&zone_config).map_err(|e| anyhow::anyhow!("zones config error: {}", e))?,
    );

    // Audit log at ~/.agent007/audit/audit.log
    let audit_dir = home.join("audit");
    let audit_log_path = audit_dir.join("audit.log");
    let audit_logger = Arc::new(AuditLogger::new(&audit_log_path));

    // ToolExecutor wired with zone checker + audit logger
    let tool_executor = Arc::new(
        ToolExecutor::new("OrchestratorAgent")
            .with_zone_checker(zone_checker.clone())
            .with_audit_logger(audit_logger.clone())
            .with_dispatcher(dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>)
            .with_mcp_client(mcp_client.clone()),
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
        run_store,
        memory_store,
        hook_executor,
        mcp_client,
        feedback_collector,
        learning_dispatcher,
        model_router,
        skill_executor,
        rag_warmup_indexed_docs,
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
    config: &Config,
    vectordb_path: &str,
    memory_store: &Arc<MemoryStore>,
    skills_dir: &std::path::Path,
    is_dry_run: bool,
) -> Result<(SkillExecutor, usize)> {
    let rag_config = config
        .memory
        .as_ref()
        .and_then(|memory| memory.rag.as_ref());

    // Prefer configured embeddings when available, but never let missing local
    // embedding support break skills/workflows. Zero-vector fallback still
    // enables keyword-based memory retrieval via Retriever.
    let (embedder, embed_dim): (Arc<dyn EmbeddingProvider>, usize) = match rag_config {
        Some(rag) if !rag.enabled => {
            let ep = MockProvider::with_embedding_dim("", "mock-embed", 384);
            (Arc::new(ep) as Arc<dyn EmbeddingProvider>, 384)
        }
        Some(rag)
            if rag.embedding_provider.eq_ignore_ascii_case("ollama")
                && config.models.ollama.is_some() =>
        {
            let ollama = config.models.ollama.as_ref().unwrap();
            let primary = Arc::new(OllamaEmbeddingProvider::new(
                &ollama.base_url,
                &rag.embedding_model,
            )) as Arc<dyn EmbeddingProvider>;
            (
                Arc::new(ResilientEmbeddingProvider::new(primary, 768))
                    as Arc<dyn EmbeddingProvider>,
                768,
            )
        }
        _ if config.models.ollama.is_some() => {
            let ollama = config.models.ollama.as_ref().unwrap();
            let primary = Arc::new(OllamaEmbeddingProvider::new(
                &ollama.base_url,
                "nomic-embed-text",
            )) as Arc<dyn EmbeddingProvider>;
            (
                Arc::new(ResilientEmbeddingProvider::new(primary, 768))
                    as Arc<dyn EmbeddingProvider>,
                768,
            )
        }
        _ => {
            let ep = MockProvider::with_embedding_dim("", "mock-embed", 384);
            (Arc::new(ep) as Arc<dyn EmbeddingProvider>, 384)
        }
    };

    // Try to build LanceDB; in dry-run, fall back to a no-op VectorDB if it fails.
    let db: Arc<dyn agent007_memory::VectorDB> = if is_dry_run {
        Arc::new(NoOpVectorDB)
    } else {
        let store = LanceDBStore::new(vectordb_path, "skills", embed_dim)
            .await
            .map_err(|e| anyhow::anyhow!("failed to open LanceDB at {}: {}", vectordb_path, e))?;
        Arc::new(store)
    };

    let rag_enabled = rag_config.map(|rag| rag.enabled).unwrap_or(true);
    let mut indexed_docs = 0usize;
    if !is_dry_run && rag_enabled {
        let indexer = Indexer::new(Arc::clone(&embedder), Arc::clone(&db), 900);
        match warmup_retrieval_index(config, memory_store, skills_dir, &indexer).await {
            Ok(count) => {
                indexed_docs = count;
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "RAG warmup index failed; continuing with keyword fallback"
                );
            }
        }
    }

    let retriever =
        Arc::new(Retriever::new(embedder, db, 5).with_memory_store(Arc::clone(memory_store)));
    let memory = memory_store.global();
    let global_store = Arc::new(agent007_memory::store::MemoryStore::new(
        agent007_global_home().join("memory"),
    ));
    let global_memory = global_store.scoped("global");

    Ok((
        SkillExecutor::new(provider, retriever, memory).with_global_memory(global_memory),
        indexed_docs,
    ))
}

const RAG_INDEX_MAX_FILES: usize = 400;
const RAG_INDEX_MAX_FILE_BYTES: u64 = 256 * 1024;
const RAG_INDEX_MAX_CHARS: usize = 80_000;
const RAG_INDEX_MAX_MEMORY_ENTRIES_PER_SCOPE: usize = 200;

fn rag_warmup_enabled() -> bool {
    std::env::var("AGENT007_RAG_WARMUP")
        .map(|raw| {
            !matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off"
            )
        })
        .unwrap_or(true)
}

async fn warmup_retrieval_index(
    config: &Config,
    memory_store: &Arc<MemoryStore>,
    skills_dir: &std::path::Path,
    indexer: &Indexer,
) -> Result<usize> {
    if !rag_warmup_enabled() {
        return Ok(0);
    }

    let mut indexed_docs = 0usize;
    indexed_docs += index_memory_scope(indexer, memory_store, "project").await?;
    indexed_docs += index_memory_scope(indexer, memory_store, "skills").await?;

    let global_store = Arc::new(MemoryStore::new(agent007_global_home().join("memory")));
    indexed_docs += index_memory_scope(indexer, &global_store, "global").await?;

    indexed_docs += index_skill_templates(indexer, skills_dir).await?;
    indexed_docs += index_configured_paths(indexer, config).await?;

    tracing::debug!(indexed_docs, "RAG warmup completed");
    Ok(indexed_docs)
}

async fn index_memory_scope(
    indexer: &Indexer,
    store: &Arc<MemoryStore>,
    scope: &str,
) -> Result<usize> {
    let scoped = store.scoped(scope);
    let keys = scoped.list_keys().unwrap_or_default();
    let mut indexed = 0usize;

    for key in keys
        .into_iter()
        .take(RAG_INDEX_MAX_MEMORY_ENTRIES_PER_SCOPE)
    {
        let Ok(Some(value)) = scoped.read(&key) else {
            continue;
        };
        let content = truncate_chars(&value, RAG_INDEX_MAX_CHARS);
        if content.trim().is_empty() {
            continue;
        }
        let doc_id = format!("memory:{scope}:{key}");
        indexer
            .index_text(&doc_id, &content)
            .await
            .map_err(|e| anyhow::anyhow!("failed to index {doc_id}: {}", e))?;
        indexed += 1;
    }

    Ok(indexed)
}

async fn index_skill_templates(indexer: &Indexer, skills_dir: &std::path::Path) -> Result<usize> {
    if !skills_dir.exists() {
        return Ok(0);
    }
    let loader = agent007_skills::SkillLoader::new(skills_dir);
    let skills = loader
        .load_all()
        .map_err(|e| anyhow::anyhow!("failed to load skills for indexing: {}", e))?;

    let mut indexed = 0usize;
    for skill in skills {
        let prompt = skill.template();
        if prompt.trim().is_empty() {
            continue;
        }
        let doc_id = format!("skill:{}", skill.trigger());
        indexer
            .index_text(&doc_id, &truncate_chars(prompt, RAG_INDEX_MAX_CHARS))
            .await
            .map_err(|e| anyhow::anyhow!("failed to index {doc_id}: {}", e))?;
        indexed += 1;
    }
    Ok(indexed)
}

async fn index_configured_paths(indexer: &Indexer, config: &Config) -> Result<usize> {
    let Some(rag) = config.memory.as_ref().and_then(|m| m.rag.as_ref()) else {
        return Ok(0);
    };
    if !rag.enabled || rag.index.is_empty() {
        return Ok(0);
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut files = Vec::new();
    for configured in &rag.index {
        let root = resolve_index_root(&cwd, configured);
        collect_indexable_files(&root, &mut files, RAG_INDEX_MAX_FILES);
        if files.len() >= RAG_INDEX_MAX_FILES {
            break;
        }
    }
    files.sort();
    files.dedup();
    files.truncate(RAG_INDEX_MAX_FILES);

    let mut indexed = 0usize;
    for file in files {
        let Ok(meta) = std::fs::metadata(&file) else {
            continue;
        };
        if meta.len() > RAG_INDEX_MAX_FILE_BYTES {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&file) else {
            continue;
        };
        let content = truncate_chars(&content, RAG_INDEX_MAX_CHARS);
        if content.trim().is_empty() {
            continue;
        }
        let rel = file
            .strip_prefix(&cwd)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| file.display().to_string());
        let doc_id = format!("file:{rel}");
        indexer
            .index_text(&doc_id, &content)
            .await
            .map_err(|e| anyhow::anyhow!("failed to index {doc_id}: {}", e))?;
        indexed += 1;
    }

    Ok(indexed)
}

fn resolve_index_root(cwd: &std::path::Path, configured: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(configured);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn collect_indexable_files(
    root: &std::path::Path,
    out: &mut Vec<std::path::PathBuf>,
    limit: usize,
) {
    if out.len() >= limit {
        return;
    }
    if !root.exists() {
        return;
    }
    let Ok(meta) = std::fs::metadata(root) else {
        return;
    };
    if meta.is_file() {
        if is_indexable_source_file(root) {
            out.push(root.to_path_buf());
        }
        return;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= limit {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_indexable_files(&path, out, limit);
        } else if is_indexable_source_file(&path) {
            out.push(path);
        }
    }
}

fn is_indexable_source_file(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    matches!(
        ext,
        "rs" | "md"
            | "txt"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "vue"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "swift"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "sql"
    )
}

fn truncate_chars(input: &str, limit: usize) -> String {
    if input.chars().count() <= limit {
        return input.to_string();
    }
    input.chars().take(limit).collect()
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

fn persist_task_memory(
    memory_store: &Arc<MemoryStore>,
    run_id: &str,
    task: &str,
    success: bool,
    output: &str,
) -> Result<()> {
    let scoped = memory_store.scoped("project");
    let record = serde_json::json!({
        "run_id": run_id,
        "task": task,
        "success": success,
        "output": output,
        "timestamp": Utc::now().to_rfc3339(),
    });
    let serialized = serde_json::to_string(&record)?;
    scoped.write("task_last", &serialized)?;
    scoped.write(&format!("task_runs/{run_id}"), &serialized)?;
    Ok(())
}

/// After a run finishes, analyze the events log and write a compact procedural
/// memory insight recording which skills were used and whether the task succeeded.
/// Insights expire after 30 days to avoid polluting long-term memory.
fn generate_auto_insights(
    memory_store: &Arc<MemoryStore>,
    run_store: &Arc<RunStore>,
    run_id: &str,
    task: &str,
    success: bool,
) {
    let Ok(run) = run_store.load_run(run_id) else {
        return;
    };

    // Collect distinct skill names from TaskCompleted events
    let mut skills_used: Vec<String> = run
        .entries
        .iter()
        .filter(|e| e.kind == "agent-event")
        .filter_map(|e| serde_json::from_value::<AgentEvent>(e.payload.clone()).ok())
        .filter_map(|ev| {
            if let AgentEvent::TaskCompleted {
                skill_name: Some(skill),
                ..
            } = ev
            {
                Some(skill)
            } else {
                None
            }
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    skills_used.sort();

    if skills_used.is_empty() && success {
        // Nothing interesting enough to record without skill attribution
        return;
    }

    let status = if success { "succeeded" } else { "failed" };
    let skills_str = if skills_used.is_empty() {
        "none".to_string()
    } else {
        skills_used.join(", ")
    };
    let truncated_task = if task.len() > 120 { &task[..120] } else { task };
    let insight = format!(
        "Task: {truncated_task}\nStatus: {status}\nSkills: {skills_str}\nRun ID: {run_id}\n"
    );

    let meta = MemoryMeta {
        entry_type: MemoryEntryType::Procedural,
        expires_after: Some("30d".to_string()),
        summary: format!("{status}: {truncated_task}"),
        ..MemoryMeta::default()
    };

    let scoped = memory_store.scoped("project");
    if let Err(e) = scoped.write_with_meta(&format!("insights:run-{run_id}"), &insight, meta) {
        tracing::warn!("auto-insights write failed: {}", e);
    }
}

pub async fn execute(config: Arc<Config>, task: String) -> Result<()> {
    let stack = build_stack(&config).await?;
    let mode = runtime_mode_label(&config);
    let provider = stack.model_router.route("task").name().to_string();
    let run = stack
        .run_store
        .create_run("task", &task, mode, Some(provider.as_str()))?;
    let _trace = stack
        .run_store
        .spawn_dispatcher_trace(
            run.id.clone(),
            stack.dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        )
        .await?;
    let _ = stack.run_store.write_json_artifact(
        &run.id,
        "retrieval-telemetry.json",
        &serde_json::json!({
            "indexed_docs": stack.rag_warmup_indexed_docs,
            "retrieval_queries": 0,
            "retrieval_hits": 0,
            "retrieval_hit_rate": 0.0,
            "rag_context_chars": 0,
            "vector_hits": 0,
            "fallback_hits": 0,
            "mock_embedding": false,
        }),
    );

    // Spawn feedback collector
    let collector = stack.feedback_collector.clone();
    stack.tracker.spawn(async move {
        if let Err(e) = collector.run().await {
            tracing::warn!("feedback collector error: {}", e);
        }
    });

    // In dry-run or non-interactive shells, skip the TUI and execute synchronously
    // so scripted usage still works and the run trace is persisted before returning.
    if should_use_non_interactive_mode() {
        let agent_task = Task::new(&task);
        match stack.orchestrator.run(agent_task).await {
            Ok(result) => {
                let _ = stack.run_store.finish_run(&run.id, true, &result.output);
                if let Err(error) =
                    persist_task_memory(&stack.memory_store, &run.id, &task, true, &result.output)
                {
                    tracing::warn!("failed to persist task memory: {}", error);
                }
                generate_auto_insights(&stack.memory_store, &stack.run_store, &run.id, &task, true);
                tracing::info!("task completed: {}", result.output);
                if !is_dry_run() {
                    println!("{}", result.output);
                }
            }
            Err(error) => {
                let _ = stack
                    .run_store
                    .finish_run(&run.id, false, error.to_string());
                if let Err(persist_error) = persist_task_memory(
                    &stack.memory_store,
                    &run.id,
                    &task,
                    false,
                    &error.to_string(),
                ) {
                    tracing::warn!("failed to persist task memory: {}", persist_error);
                }
                generate_auto_insights(
                    &stack.memory_store,
                    &stack.run_store,
                    &run.id,
                    &task,
                    false,
                );
                return Err(error.into());
            }
        }
        stack.cancel.cancel();
        stack.tracker.close();
        return Ok(());
    }

    // Submit the task to the orchestrator
    let orchestrator = stack.orchestrator.clone();
    let run_store = stack.run_store.clone();
    let memory_store = stack.memory_store.clone();
    let run_id = run.id.clone();
    let task_desc = task.clone();
    stack.tracker.spawn(async move {
        let agent_task = Task::new(&task_desc);
        match orchestrator.run(agent_task).await {
            Ok(result) => {
                let _ = run_store.finish_run(&run_id, true, &result.output);
                if let Err(error) =
                    persist_task_memory(&memory_store, &run_id, &task_desc, true, &result.output)
                {
                    tracing::warn!("failed to persist task memory: {}", error);
                }
                generate_auto_insights(&memory_store, &run_store, &run_id, &task_desc, true);
                tracing::info!("task completed: {}", result.output);
            }
            Err(e) => {
                let _ = run_store.finish_run(&run_id, false, e.to_string());
                if let Err(error) =
                    persist_task_memory(&memory_store, &run_id, &task_desc, false, &e.to_string())
                {
                    tracing::warn!("failed to persist task memory: {}", error);
                }
                generate_auto_insights(&memory_store, &run_store, &run_id, &task_desc, false);
                tracing::warn!("task failed: {}", e);
            }
        }
    });

    // Construct App and EventLoop
    let mut app = App::default();
    app.push_log(format!("Starting task: {}", task));
    app.push_log(format!("Run ID: {}", run.id));

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
    use crate::test_support::env_lock;
    use agent007_models::ModelError;

    struct FailingEmbedder;

    #[async_trait::async_trait]
    impl EmbeddingProvider for FailingEmbedder {
        fn name(&self) -> &str {
            "failing-embedder"
        }

        async fn embed(&self, _text: &str) -> Result<Vec<f32>, ModelError> {
            Err(ModelError::Api {
                provider: "failing-embedder".to_string(),
                message: "HTTP 404".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn run_command_builds_stack_without_panic() {
        let _guard = env_lock();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Config::default();
        let stack = build_stack(&config).await.unwrap();
        // Verify stack was constructed (just check fields exist)
        assert!(stack.cancel.is_cancelled() == false);
        std::env::remove_var("AGENT007_DRY_RUN");
    }

    #[tokio::test]
    async fn build_stack_contains_persona_registry_with_builtins() {
        let _guard = env_lock();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Config::default();
        let stack = build_stack(&config).await.unwrap();
        use agent007_core::PersonaProvider;
        assert!(stack.persona_registry.list().len() >= 15);
        std::env::remove_var("AGENT007_DRY_RUN");
    }

    #[tokio::test]
    async fn e2e_smoke_run_with_dry_run() {
        let _guard = env_lock();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Arc::new(Config::default());
        let result = execute(config, "say hello".to_string()).await;
        std::env::remove_var("AGENT007_DRY_RUN");
        assert!(result.is_ok(), "run command failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn stack_contains_workflow_runner() {
        let _guard = env_lock();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Config::default();
        let stack = build_stack(&config).await.unwrap();
        // WorkflowRunner is on the stack — test that it can validate an empty-step workflow
        use agent007_workflows::types::WorkflowDef;
        let def = WorkflowDef {
            name: "t".to_string(),
            description: None,
            steps: vec![],
            budget: None,
            reliability: None,
            eval_gate: None,
        };
        let result = stack.workflow_runner.validate(&def);
        // Empty workflow validates to empty batches
        assert!(result.is_ok());
        std::env::remove_var("AGENT007_DRY_RUN");
    }

    #[tokio::test]
    async fn build_stack_creates_workflows_dir() {
        let _guard = env_lock();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path().to_str().unwrap());
        let config = Config::default();
        let _ = build_stack(&config).await.unwrap();
        assert!(tmp.path().join("workflows").exists());
        std::env::remove_var("AGENT007_HOME");
        std::env::remove_var("AGENT007_DRY_RUN");
    }

    #[tokio::test]
    async fn execute_persists_task_memory_records() {
        let _guard = env_lock();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path().to_str().unwrap());

        let config = Arc::new(Config::default());
        execute(config, "persist this task".to_string())
            .await
            .unwrap();

        let task_last = tmp
            .path()
            .join("memory")
            .join("project")
            .join("task_last.md");
        assert!(task_last.exists(), "task_last memory record should exist");

        let store = Arc::new(agent007_memory::store::MemoryStore::new(
            tmp.path().join("memory"),
        ));
        let content = store.scoped("project").read("task_last").unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(json["task"], "persist this task");
        assert!(json["run_id"].as_str().is_some());
        assert!(json["timestamp"].as_str().is_some());

        std::env::remove_var("AGENT007_HOME");
        std::env::remove_var("AGENT007_DRY_RUN");
    }

    #[test]
    fn no_tui_env_forces_non_interactive_mode() {
        let _guard = env_lock();
        std::env::set_var("AGENT007_NO_TUI", "1");
        assert!(should_use_non_interactive_mode());
        std::env::remove_var("AGENT007_NO_TUI");
    }

    #[tokio::test]
    async fn resilient_embedding_provider_returns_zero_vector_on_failure() {
        let provider = ResilientEmbeddingProvider::new(Arc::new(FailingEmbedder), 7);
        let embedding = provider.embed("hello").await.unwrap();
        assert_eq!(embedding, vec![0.0; 7]);
    }

    #[test]
    fn indexable_source_file_matches_expected_extensions() {
        assert!(is_indexable_source_file(std::path::Path::new(
            "src/main.rs"
        )));
        assert!(is_indexable_source_file(std::path::Path::new(
            "docs/plan.md"
        )));
        assert!(is_indexable_source_file(std::path::Path::new(
            "web/app.vue"
        )));
        assert!(!is_indexable_source_file(std::path::Path::new(
            "assets/logo.png"
        )));
        assert!(!is_indexable_source_file(std::path::Path::new(
            "bin/agent007"
        )));
    }

    #[test]
    fn collect_indexable_files_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(dir.path().join("b.md"), "# b").unwrap();
        std::fs::write(dir.path().join("c.toml"), "[x]").unwrap();
        std::fs::write(dir.path().join("d.png"), "not text").unwrap();

        let mut files = Vec::new();
        collect_indexable_files(dir.path(), &mut files, 2);
        assert!(files.len() <= 2);
    }
}
