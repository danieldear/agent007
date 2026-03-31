use std::sync::Arc;
use agent007_memory::{Retriever, ScopedMemoryStore};
use agent007_models::{CompletionRequest, Message, ModelProvider, Role};
use crate::error::SkillError;
use crate::types::Skill;

pub struct SkillExecutor {
    provider: Arc<dyn ModelProvider>,
    retriever: Arc<Retriever>,
    memory: ScopedMemoryStore,
}

impl SkillExecutor {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        retriever: Arc<Retriever>,
        memory: ScopedMemoryStore,
    ) -> Self {
        Self { provider, retriever, memory }
    }

    pub async fn execute(&self, skill: &Skill, args: &str) -> Result<String, SkillError> {
        // 1. RAG context
        let rag_context = self.retriever.retrieve(args).await
            .map_err(|e| SkillError::Memory { name: skill.name().to_string(), source: e })?;

        // 2. Read memory values — data lives at <scope>/<key>.md, so read the full scope
        let memory_user = self.memory.inner.scoped("user").read_all()
            .map_err(|e| SkillError::Memory { name: skill.name().to_string(), source: e })?;
        let memory_project = self.memory.inner.scoped("project").read_all()
            .map_err(|e| SkillError::Memory { name: skill.name().to_string(), source: e })?;

        // 3. Build Tera context
        let mut ctx = tera::Context::new();
        ctx.insert("args", args);
        ctx.insert("task", args);
        ctx.insert("rag_context", &rag_context);
        ctx.insert("memory", &serde_json::json!({
            "user": memory_user,
            "project": memory_project,
        }));
        ctx.insert("date", &chrono::Utc::now().format("%Y-%m-%d").to_string());

        // 4. Render template (autoescape = false to avoid HTML-escaping memory content)
        let rendered = tera::Tera::one_off(skill.template(), &ctx, false)
            .map_err(|e| SkillError::TemplateRender { name: skill.name().to_string(), source: e })?;

        // 5. Call model with skill's specified model
        let request = CompletionRequest {
            model: skill.model().to_string(),
            messages: vec![Message { role: Role::User, content: rendered }],
            max_tokens: None,
            temperature: None,
            system: None,
        };

        let response = self.provider.complete(request).await
            .map_err(|e| SkillError::Model { name: skill.name().to_string(), source: e })?;

        Ok(response.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_memory::{MemoryError, MemoryStore, Retriever, SearchResult, VectorDB};
    use agent007_models::{EmbeddingProvider, ModelError, ModelProvider, CompletionRequest, CompletionResponse};
    use async_trait::async_trait;
    use crate::types::{Skill, SkillFrontmatter};
    use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
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
        fn call_count(&self) -> usize { self.calls.load(Ordering::SeqCst) }
        fn last_model(&self) -> Option<String> { self.last_model.lock().unwrap().clone() }
    }
    #[async_trait]
    impl ModelProvider for MockModelProvider {
        async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_model.lock().unwrap() = Some(request.model.clone());
            Ok(CompletionResponse {
                content: "mock-output".to_string(),
                model: request.model,
                input_tokens: None,
                output_tokens: None,
            })
        }
        fn name(&self) -> &str { "mock" }
    }

    struct FixedEmbedder;
    #[async_trait]
    impl EmbeddingProvider for FixedEmbedder {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, ModelError> {
            Ok(vec![0.0; 4])
        }
        fn name(&self) -> &str { "fixed" }
    }

    struct FixedVectorDB { fragment: String }
    #[async_trait]
    impl VectorDB for FixedVectorDB {
        async fn upsert(&self, _id: &str, _v: Vec<f32>, _p: serde_json::Value) -> Result<(), MemoryError> { Ok(()) }
        async fn search(&self, _q: Vec<f32>, _limit: usize) -> Result<Vec<SearchResult>, MemoryError> {
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
            },
            template: template.to_string(),
        }
    }

    fn make_executor(dir: &std::path::Path, provider: Arc<MockModelProvider>) -> SkillExecutor {
        let embedder = Arc::new(FixedEmbedder) as Arc<dyn EmbeddingProvider>;
        let db = Arc::new(FixedVectorDB { fragment: "rag-fragment".to_string() }) as Arc<dyn VectorDB>;
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
        let skill = make_skill("User: {{memory.user}} RAG: {{rag_context}} Args: {{args}}", "claude");
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
}
