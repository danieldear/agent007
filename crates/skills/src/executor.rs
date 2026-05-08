use crate::error::SkillError;
use crate::types::Skill;
use agent007_lsp_client::LspClient;
use agent007_memory::retriever::RetrieveStats;
use agent007_memory::{Retriever, ScopedMemoryStore};
use agent007_models::{CompletionRequest, Message, ModelProvider, ModelRouter, Role};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillExecutionMetrics {
    pub retrieval_queries: u32,
    pub retrieval_hits: u32,
    pub retrieval_hit_rate: f64,
    pub rag_context_chars: usize,
    pub vector_hits: usize,
    pub fallback_hits: usize,
    pub mock_embedding: bool,
    /// Actual token counts from the LLM API response, if available.
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
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
            lsp_inject_categories: vec!["code_completion".to_string(), "reasoning".to_string()],
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
        // 1. RAG context
        let (rag_context, retrieval_stats) = self
            .retriever
            .retrieve_with_stats(args)
            .await
            .map_err(|e| SkillError::Memory {
                name: skill.name().to_string(),
                source: e,
            })?;

        // 2. Read memory values — data lives at <scope>/<key>.md, so read the full scope
        let memory_user =
            self.memory
                .inner
                .scoped("user")
                .read_all()
                .map_err(|e| SkillError::Memory {
                    name: skill.name().to_string(),
                    source: e,
                })?;
        let memory_project = self
            .memory
            .inner
            .scoped("project")
            .read_all()
            .map_err(|e| SkillError::Memory {
                name: skill.name().to_string(),
                source: e,
            })?;
        let memory_global = match &self.global_memory {
            Some(store) => store.read_all().map_err(|e| SkillError::Memory {
                name: skill.name().to_string(),
                source: e,
            })?,
            None => String::new(),
        };

        // 3. LSP context — auto-detect language server and inject diagnostics/symbols
        let lsp_context_str = if self
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
                    .map(|ctx| ctx.to_prompt_string())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        } else {
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
        metrics.input_tokens = response.input_tokens;
        metrics.output_tokens = response.output_tokens;

        Ok(SkillExecutionReport {
            output: response.content,
            metrics,
        })
    }
}

fn metrics_from_retrieval(rag_context: &str, retrieval: &RetrieveStats) -> SkillExecutionMetrics {
    let hits = u32::from(!rag_context.is_empty());
    SkillExecutionMetrics {
        retrieval_queries: 1,
        retrieval_hits: hits,
        retrieval_hit_rate: hits as f64,
        rag_context_chars: rag_context.chars().count(),
        vector_hits: retrieval.vector_hits,
        fallback_hits: retrieval.fallback_hits,
        mock_embedding: retrieval.mock_embedding,
        input_tokens: None,
        output_tokens: None,
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
    }
    impl MockModelProvider {
        fn new() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                last_model: Arc::new(Mutex::new(None)),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        fn last_model(&self) -> Option<String> {
            self.last_model.lock().unwrap().clone()
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
            Ok(CompletionResponse {
                content: "mock-output".to_string(),
                model: request.model,
                input_tokens: None,
                output_tokens: None,
                cached_tokens: None,
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
}
