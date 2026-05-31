use crate::error::SkillError;
use crate::types::Skill;
use agent007_lsp_client::LspClient;
use agent007_memory::retriever::RetrieveStats;
use agent007_memory::{Retriever, ScopedMemoryStore};
use agent007_models::{CompletionRequest, Message, ModelProvider, ModelRouter, Role};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const MAX_SKILL_RAG_CONTEXT_CHARS: usize = 8_000;
const MAX_SKILL_MEMORY_CONTEXT_CHARS: usize = 6_000;
const MAX_SKILL_LSP_CONTEXT_CHARS: usize = 6_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillExecutionMetrics {
    pub retrieval_queries: u32,
    pub retrieval_hits: u32,
    pub retrieval_hit_rate: f64,
    pub rag_context_chars: usize,
    pub memory_context_chars: usize,
    pub lsp_context_chars: usize,
    pub rendered_prompt_chars: usize,
    pub skipped_context_sections: Vec<String>,
    pub context_policy: String,
    pub graph_hits: usize,
    pub graph_files: usize,
    pub graph_context_chars: usize,
    pub vector_hits: usize,
    pub fallback_hits: usize,
    pub mock_embedding: bool,
    /// Actual token counts from the LLM API response, if available.
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cache_read_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
    pub total_tokens: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillExecutionReport {
    pub output: String,
    pub metrics: SkillExecutionMetrics,
}

pub struct SkillExecutor {
    provider: Arc<dyn ModelProvider>,
    retriever: Arc<Retriever>,
    memory: ScopedMemoryStore,
    global_memory: Option<ScopedMemoryStore>,
    router: Option<Arc<ModelRouter>>,
    lsp_inject_categories: Vec<String>,
}

impl SkillExecutor {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        retriever: Arc<Retriever>,
        memory: ScopedMemoryStore,
    ) -> Self {
        Self {
            provider,
            retriever,
            memory,
            global_memory: None,
            router: None,
            lsp_inject_categories: ["code_completion", "reasoning", "code", "dev", "frontend"]
                .into_iter()
                .map(ToString::to_string)
                .collect(),
        }
    }

    pub fn with_global_memory(mut self, global: ScopedMemoryStore) -> Self {
        self.global_memory = Some(global);
        self
    }

    pub fn with_router(mut self, router: Arc<ModelRouter>) -> Self {
        self.router = Some(router);
        self
    }

    pub fn with_lsp_categories(mut self, categories: Vec<String>) -> Self {
        self.lsp_inject_categories = categories;
        self
    }

    pub async fn execute(&self, skill: &Skill, args: &str) -> Result<String, SkillError> {
        let report = self.execute_with_report(skill, args).await?;
        Ok(report.output)
    }

    pub async fn execute_with_report(
        &self,
        skill: &Skill,
        args: &str,
    ) -> Result<SkillExecutionReport, SkillError> {
        let needs = TemplateContextNeeds::from_template(skill.template());
        let mut skipped_context_sections = Vec::new();

        // 1. RAG context: load lazily only when the template actually asks for it.
        let (rag_context, retrieval_stats) = if needs.rag_context {
            let (context, stats) = self
                .retriever
                .retrieve_with_stats(args)
                .await
                .map_err(|e| SkillError::Memory {
                    name: skill.name().to_string(),
                    source: e,
                })?;
            (
                cap_context_block("rag_context", &context, MAX_SKILL_RAG_CONTEXT_CHARS),
                stats,
            )
        } else {
            skipped_context_sections.push("rag_context".to_string());
            (String::new(), RetrieveStats::default())
        };

        // 2. Read memory lazily and cap each block. This avoids paying for full
        // memory scopes on skills that only need args/task or only need one scope.
        let memory_user = if needs.memory_user {
            let scoped = self.memory.inner.scoped("user");
            cap_context_block(
                "memory.user",
                &scoped.read_top_n(4).map_err(|e| SkillError::Memory {
                    name: skill.name().to_string(),
                    source: e,
                })?,
                MAX_SKILL_MEMORY_CONTEXT_CHARS,
            )
        } else {
            skipped_context_sections.push("memory.user".to_string());
            String::new()
        };
        let memory_project = if needs.memory_project {
            let scoped = self.memory.inner.scoped("project");
            cap_context_block(
                "memory.project",
                &scoped.read_top_n(8).map_err(|e| SkillError::Memory {
                    name: skill.name().to_string(),
                    source: e,
                })?,
                MAX_SKILL_MEMORY_CONTEXT_CHARS,
            )
        } else {
            skipped_context_sections.push("memory.project".to_string());
            String::new()
        };
        let memory_global = if needs.memory_global {
            match &self.global_memory {
                Some(store) => cap_context_block(
                    "memory.global",
                    &store.read_top_n(4).map_err(|e| SkillError::Memory {
                        name: skill.name().to_string(),
                        source: e,
                    })?,
                    MAX_SKILL_MEMORY_CONTEXT_CHARS,
                ),
                None => String::new(),
            }
        } else {
            skipped_context_sections.push("memory.global".to_string());
            String::new()
        };

        // 3. LSP context — only query when the template asks for it and the skill
        // category is configured for LSP enrichment.
        let lsp_context_str = if needs.lsp_context
            && self
                .lsp_inject_categories
                .iter()
                .any(|c| c == skill.category())
        {
            let cwd = std::env::current_dir().unwrap_or_default();
            if let Some((_lang, server_cmd)) = LspClient::detect_language(&cwd) {
                let client = LspClient::new(server_cmd);
                client
                    .query(&cwd, &[])
                    .await
                    .map(|ctx| {
                        cap_context_block(
                            "lsp_context",
                            &ctx.to_prompt_string(),
                            MAX_SKILL_LSP_CONTEXT_CHARS,
                        )
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            }
        } else {
            skipped_context_sections.push("lsp_context".to_string());
            String::new()
        };

        // 4. Build Tera context
        let mut ctx = tera::Context::new();
        ctx.insert("args", args);
        ctx.insert("task", args);
        ctx.insert("rag_context", &rag_context);
        ctx.insert("lsp_context", &lsp_context_str);
        ctx.insert(
            "skill_dir",
            &skill.skill_dir().to_string_lossy().to_string(),
        );
        ctx.insert(
            "memory",
            &serde_json::json!({
                "user": memory_user,
                "project": memory_project,
                "global": memory_global,
            }),
        );
        ctx.insert("date", &chrono::Utc::now().format("%Y-%m-%d").to_string());

        // 4. Render template (autoescape = false to avoid HTML-escaping memory content)
        let rendered = tera::Tera::one_off(skill.template(), &ctx, false).map_err(|e| {
            SkillError::TemplateRender {
                name: skill.name().to_string(),
                source: e,
            }
        })?;

        let rendered_prompt_chars = rendered.chars().count();
        let request = CompletionRequest {
            model: skill.model().to_string(),
            messages: vec![Message {
                role: Role::User,
                content: rendered,
            }],
            max_tokens: None,
            temperature: None,
            system: None,
        };

        let response = if let Some(router) = &self.router {
            router
                .complete_for_task_type(skill.category(), request)
                .await
        } else {
            self.provider.complete(request).await
        }
        .map_err(|e| SkillError::Model {
            name: skill.name().to_string(),
            source: e,
        })?;

        let mut metrics = metrics_from_retrieval(&rag_context, &retrieval_stats);
        metrics.memory_context_chars = memory_user.chars().count()
            + memory_project.chars().count()
            + memory_global.chars().count();
        metrics.lsp_context_chars = lsp_context_str.chars().count();
        metrics.rendered_prompt_chars = rendered_prompt_chars;
        metrics.skipped_context_sections = skipped_context_sections;
        metrics.context_policy = "placeholder-aware-capped".to_string();
        metrics.input_tokens = response.input_tokens;
        metrics.output_tokens = response.output_tokens;
        metrics.cache_read_tokens = response.cached_tokens;
        metrics.cache_write_tokens = response.cache_write_tokens;
        metrics.total_tokens = response.total_tokens_with_fallback();
        metrics.estimated_cost_usd = response.estimated_cost_usd;

        Ok(SkillExecutionReport {
            output: response.content,
            metrics,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct TemplateContextNeeds {
    rag_context: bool,
    memory_user: bool,
    memory_project: bool,
    memory_global: bool,
    lsp_context: bool,
}

impl TemplateContextNeeds {
    fn from_template(template: &str) -> Self {
        let compact: String = template.chars().filter(|c| !c.is_whitespace()).collect();
        Self {
            rag_context: compact.contains("{{rag_context") || template.contains("rag_context"),
            memory_user: compact.contains("memory.user"),
            memory_project: compact.contains("memory.project"),
            memory_global: compact.contains("memory.global"),
            lsp_context: compact.contains("{{lsp_context") || template.contains("lsp_context"),
        }
    }
}

fn cap_context_block(label: &str, value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push_str(&format!(
        "\n\n[agent007-context truncated: {label}; original_chars={}; kept_chars={max_chars}]",
        value.chars().count()
    ));
    truncated
}

fn metrics_from_retrieval(rag_context: &str, retrieval: &RetrieveStats) -> SkillExecutionMetrics {
    let queried = retrieval.query_chars > 0;
    let hits = u32::from(!rag_context.is_empty());
    SkillExecutionMetrics {
        retrieval_queries: u32::from(queried),
        retrieval_hits: hits,
        retrieval_hit_rate: if queried { hits as f64 } else { 0.0 },
        rag_context_chars: rag_context.chars().count(),
        memory_context_chars: 0,
        lsp_context_chars: 0,
        rendered_prompt_chars: 0,
        skipped_context_sections: Vec::new(),
        context_policy: "placeholder-aware-capped".to_string(),
        graph_hits: retrieval.graph_hits,
        graph_files: retrieval.graph_files,
        graph_context_chars: retrieval.graph_context_chars,
        vector_hits: retrieval.vector_hits,
        fallback_hits: retrieval.fallback_hits,
        mock_embedding: retrieval.mock_embedding,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        total_tokens: None,
        estimated_cost_usd: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Skill, SkillFrontmatter};
    use agent007_memory::{MemoryError, MemoryStore, Retriever, SearchResult, VectorDB};
    use agent007_models::{
        CompletionRequest, CompletionResponse, EmbeddingProvider, ModelError, ModelProvider,
    };
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use tempfile::TempDir;

    struct MockModelProvider {
        calls: Arc<AtomicUsize>,
        last_model: Arc<Mutex<Option<String>>>,
        last_prompt: Arc<Mutex<Option<String>>>,
    }
    impl MockModelProvider {
        fn new() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                last_model: Arc::new(Mutex::new(None)),
                last_prompt: Arc::new(Mutex::new(None)),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        fn last_model(&self) -> Option<String> {
            self.last_model.lock().unwrap().clone()
        }
        fn last_prompt(&self) -> Option<String> {
            self.last_prompt.lock().unwrap().clone()
        }
    }
    #[async_trait]
    impl ModelProvider for MockModelProvider {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, ModelError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_model.lock().unwrap() = Some(request.model.clone());
            *self.last_prompt.lock().unwrap() = request.messages.first().map(|m| m.content.clone());
            Ok(CompletionResponse {
                content: "mock-output".to_string(),
                model: request.model,
                input_tokens: None,
                output_tokens: None,
                cached_tokens: None,
                cache_write_tokens: None,
                total_tokens: None,
                estimated_cost_usd: None,
            })
        }
        fn name(&self) -> &str {
            "mock"
        }
    }

    struct FixedEmbedder;
    #[async_trait]
    impl EmbeddingProvider for FixedEmbedder {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, ModelError> {
            Ok(vec![0.0; 4])
        }
        fn name(&self) -> &str {
            "fixed"
        }
    }

    struct FixedVectorDB {
        fragment: String,
    }
    #[async_trait]
    impl VectorDB for FixedVectorDB {
        async fn upsert(
            &self,
            _id: &str,
            _v: Vec<f32>,
            _p: serde_json::Value,
        ) -> Result<(), MemoryError> {
            Ok(())
        }
        async fn search(
            &self,
            _q: Vec<f32>,
            _limit: usize,
        ) -> Result<Vec<SearchResult>, MemoryError> {
            Ok(vec![SearchResult {
                id: "x".to_string(),
                score: 1.0,
                payload: serde_json::json!({ "text": self.fragment }),
            }])
        }
    }

    fn make_skill(template: &str, model: &str) -> Skill {
        Skill {
            frontmatter: SkillFrontmatter {
                name: "test-skill".to_string(),
                description: "test".to_string(),
                trigger: "/test".to_string(),
                model: model.to_string(),
                category: "custom".to_string(),
                version: "1.0.0".to_string(),
                tags: Vec::new(),
            },
            template: template.to_string(),
            manifest_path: PathBuf::new(),
            entry_path: PathBuf::new(),
            skill_dir: PathBuf::from("/tmp/test-skill"),
        }
    }

    fn make_executor(dir: &std::path::Path, provider: Arc<MockModelProvider>) -> SkillExecutor {
        let embedder = Arc::new(FixedEmbedder) as Arc<dyn EmbeddingProvider>;
        let db = Arc::new(FixedVectorDB {
            fragment: "rag-fragment".to_string(),
        }) as Arc<dyn VectorDB>;
        let retriever = Arc::new(Retriever::new(embedder, db, 1));
        let store = Arc::new(MemoryStore::new(dir));
        let memory = store.global();
        SkillExecutor::new(provider as Arc<dyn ModelProvider>, retriever, memory)
    }

    #[tokio::test]
    async fn executor_returns_model_response() {
        let dir = TempDir::new().unwrap();
        let provider = Arc::new(MockModelProvider::new());
        let executor = make_executor(dir.path(), Arc::clone(&provider));
        let skill = make_skill(
            "User: {{memory.user}} RAG: {{rag_context}} Args: {{args}}",
            "claude",
        );
        let result = executor.execute(&skill, "hello").await.unwrap();
        assert_eq!(result, "mock-output");
    }

    #[tokio::test]
    async fn executor_calls_model_exactly_once() {
        let dir = TempDir::new().unwrap();
        let provider = Arc::new(MockModelProvider::new());
        let executor = make_executor(dir.path(), Arc::clone(&provider));
        let skill = make_skill("Args: {{args}}", "claude");
        executor.execute(&skill, "hello").await.unwrap();
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn executor_uses_skill_model_name() {
        let dir = TempDir::new().unwrap();
        let provider = Arc::new(MockModelProvider::new());
        let executor = make_executor(dir.path(), Arc::clone(&provider));
        let skill = make_skill("Args: {{args}}", "ollama");
        executor.execute(&skill, "x").await.unwrap();
        assert_eq!(provider.last_model(), Some("ollama".to_string()));
    }

    #[tokio::test]
    async fn executor_uses_router_to_normalize_foreign_model_hints() {
        let dir = TempDir::new().unwrap();
        let provider = Arc::new(MockModelProvider::new());
        let mut router = ModelRouter::new("ollama");
        router.register("ollama", Arc::clone(&provider) as Arc<dyn ModelProvider>);
        router.add_rule("custom", "ollama");
        router.alias("claude-sonnet-4-6", "claude");

        let executor =
            make_executor(dir.path(), Arc::clone(&provider)).with_router(Arc::new(router));
        let skill = make_skill("Args: {{args}}", "claude-sonnet-4-6");
        executor.execute(&skill, "plan").await.unwrap();
        assert_eq!(provider.last_model(), Some("ollama".to_string()));
    }

    #[tokio::test]
    async fn executor_injects_skill_dir_into_template_context() {
        let dir = TempDir::new().unwrap();
        let provider = Arc::new(MockModelProvider::new());
        let executor = make_executor(dir.path(), Arc::clone(&provider));
        let skill = make_skill("Skill dir: {{skill_dir}}", "claude");

        let result = executor.execute(&skill, "x").await.unwrap();
        assert_eq!(result, "mock-output");
        // Smoke assertion on render path: execution succeeded with the new variable present.
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn executor_skips_context_that_template_does_not_request() {
        let dir = TempDir::new().unwrap();
        let provider = Arc::new(MockModelProvider::new());
        let executor = make_executor(dir.path(), Arc::clone(&provider));
        let skill = make_skill("Args only: {{args}}", "claude");

        let report = executor.execute_with_report(&skill, "hello").await.unwrap();

        assert_eq!(report.metrics.retrieval_queries, 0);
        assert_eq!(report.metrics.rag_context_chars, 0);
        assert_eq!(report.metrics.memory_context_chars, 0);
        assert!(report
            .metrics
            .skipped_context_sections
            .contains(&"rag_context".to_string()));
        assert!(!provider.last_prompt().unwrap().contains("rag-fragment"));
    }

    #[tokio::test]
    async fn executor_loads_only_requested_memory_scope() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        store
            .scoped("project")
            .write("note", "project-context")
            .unwrap();
        store.scoped("user").write("note", "user-context").unwrap();

        let provider = Arc::new(MockModelProvider::new());
        let embedder = Arc::new(FixedEmbedder) as Arc<dyn EmbeddingProvider>;
        let db = Arc::new(FixedVectorDB {
            fragment: "rag-fragment".to_string(),
        }) as Arc<dyn VectorDB>;
        let retriever = Arc::new(Retriever::new(embedder, db, 1));
        let executor = SkillExecutor::new(
            Arc::clone(&provider) as Arc<dyn ModelProvider>,
            retriever,
            store.global(),
        );
        let skill = make_skill("Project: {{memory.project}}", "claude");

        let report = executor.execute_with_report(&skill, "hello").await.unwrap();
        let prompt = provider.last_prompt().unwrap();

        assert!(prompt.contains("project-context"));
        assert!(!prompt.contains("user-context"));
        assert!(report.metrics.memory_context_chars > 0);
        assert!(report
            .metrics
            .skipped_context_sections
            .contains(&"memory.user".to_string()));
    }

    #[tokio::test]
    async fn executor_includes_repo_graph_context_in_metrics() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();

        let provider = Arc::new(MockModelProvider::new());
        let embedder = Arc::new(FixedEmbedder) as Arc<dyn EmbeddingProvider>;
        let db = Arc::new(FixedVectorDB {
            fragment: "rag-fragment".to_string(),
        }) as Arc<dyn VectorDB>;
        let retriever = Arc::new(
            Retriever::new(embedder, db, 1).with_repo_graph_root(dir.path().to_path_buf()),
        );
        let store = Arc::new(MemoryStore::new(dir.path()));
        let memory = store.global();
        let executor = SkillExecutor::new(
            Arc::clone(&provider) as Arc<dyn ModelProvider>,
            retriever,
            memory,
        );
        let skill = make_skill("Args: {{args}}\n{{rag_context}}", "claude");

        let report = executor.execute_with_report(&skill, "alpha").await.unwrap();
        assert!(report.metrics.graph_hits >= 1);
        assert!(report.metrics.graph_context_chars > 0);
        assert_eq!(provider.call_count(), 1);
    }
}
