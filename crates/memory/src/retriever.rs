use crate::error::MemoryError;
use crate::store::MemoryStore;
use crate::vectordb::VectorDB;
use agent007_models::EmbeddingProvider;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetrieveStats {
    pub query_chars: usize,
    pub vector_hits: usize,
    pub fallback_hits: usize,
    pub used_vector: bool,
    pub used_fallback: bool,
    pub mock_embedding: bool,
}

pub struct Retriever {
    embedder: Arc<dyn EmbeddingProvider>,
    db: Arc<dyn VectorDB>,
    top_k: usize,
    /// Optional memory store for keyword fallback when vector search returns nothing.
    memory_store: Option<Arc<MemoryStore>>,
}

impl Retriever {
    pub fn new(embedder: Arc<dyn EmbeddingProvider>, db: Arc<dyn VectorDB>, top_k: usize) -> Self {
        Self {
            embedder,
            db,
            top_k,
            memory_store: None,
        }
    }

    /// Attach a memory store used as keyword fallback when vector search is empty.
    pub fn with_memory_store(mut self, store: Arc<MemoryStore>) -> Self {
        self.memory_store = Some(store);
        self
    }

    pub async fn retrieve(&self, query: &str) -> Result<String, MemoryError> {
        let (context, _stats) = self.retrieve_with_stats(query).await?;
        Ok(context)
    }

    pub async fn retrieve_with_stats(
        &self,
        query: &str,
    ) -> Result<(String, RetrieveStats), MemoryError> {
        let mut stats = RetrieveStats {
            query_chars: query.chars().count(),
            ..RetrieveStats::default()
        };
        let embedding = self
            .embedder
            .embed(query)
            .await
            .map_err(|e| MemoryError::Embedding(e.to_string()))?;

        // Detect mock/zero embeddings — all zeros means the embedder is a stub.
        let is_mock_embedding = embedding.iter().all(|&v| v == 0.0);
        stats.mock_embedding = is_mock_embedding;

        let fragments: Vec<String> = if is_mock_embedding {
            vec![]
        } else {
            let results = self.db.search(embedding, self.top_k).await?;
            stats.vector_hits = results.len();
            results
                .iter()
                .filter_map(|r| {
                    r.payload
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        };

        if !fragments.is_empty() {
            stats.used_vector = true;
            return Ok((fragments.join("\n\n"), stats));
        }

        // Fallback: keyword scan across scoped memory files
        if let Some(store) = &self.memory_store {
            let query_lower = query.to_lowercase();
            let keywords: Vec<&str> = query_lower.split_whitespace().collect();
            let mut matched = Vec::new();
            for scope in &["user", "project", "skills"] {
                if let Ok(keys) = store.scoped(scope).list_keys() {
                    for key in &keys {
                        if let Ok(Some(val)) = store.scoped(scope).read(key) {
                            let val_lower = val.to_lowercase();
                            if keywords
                                .iter()
                                .any(|kw| kw.len() >= 3 && val_lower.contains(kw))
                            {
                                stats.fallback_hits += 1;
                                matched.push(format!("[{}/{}]\n{}", scope, key, val));
                            }
                        }
                    }
                }
            }
            if !matched.is_empty() {
                stats.used_fallback = true;
                return Ok((matched.join("\n\n"), stats));
            }
        }

        Ok((String::new(), stats))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MemoryError;
    use crate::vectordb::{SearchResult, VectorDB};
    use agent007_models::{EmbeddingProvider, ModelError};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct MockEmbeddingProvider;
    #[async_trait]
    impl EmbeddingProvider for MockEmbeddingProvider {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, ModelError> {
            Ok(vec![0.1; 4])
        }
        fn name(&self) -> &str {
            "mock-embed"
        }
    }

    struct MockVectorDB;
    #[async_trait]
    impl VectorDB for MockVectorDB {
        async fn upsert(
            &self,
            _id: &str,
            _vector: Vec<f32>,
            _payload: serde_json::Value,
        ) -> Result<(), MemoryError> {
            Ok(())
        }
        async fn search(
            &self,
            _query: Vec<f32>,
            _limit: usize,
        ) -> Result<Vec<SearchResult>, MemoryError> {
            Ok(vec![
                SearchResult {
                    id: "a".to_string(),
                    score: 0.9,
                    payload: serde_json::json!({ "text": "fragment_alpha" }),
                },
                SearchResult {
                    id: "b".to_string(),
                    score: 0.8,
                    payload: serde_json::json!({ "text": "fragment_beta" }),
                },
            ])
        }
    }

    #[tokio::test]
    async fn retriever_returns_joined_fragments() {
        let embedder = Arc::new(MockEmbeddingProvider);
        let db = Arc::new(MockVectorDB);
        let retriever = Retriever::new(
            embedder as Arc<dyn EmbeddingProvider>,
            db as Arc<dyn VectorDB>,
            2,
        );

        let result = retriever.retrieve("some query").await.unwrap();
        assert!(
            result.contains("fragment_alpha"),
            "result should contain fragment_alpha"
        );
        assert!(
            result.contains("fragment_beta"),
            "result should contain fragment_beta"
        );
    }
}
