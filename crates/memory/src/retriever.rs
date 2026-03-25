use std::sync::Arc;
use agent007_models::EmbeddingProvider;
use crate::error::MemoryError;
use crate::vectordb::VectorDB;

pub struct Retriever {
    embedder: Arc<dyn EmbeddingProvider>,
    db: Arc<dyn VectorDB>,
    top_k: usize,
}

impl Retriever {
    pub fn new(
        embedder: Arc<dyn EmbeddingProvider>,
        db: Arc<dyn VectorDB>,
        top_k: usize,
    ) -> Self {
        Self { embedder, db, top_k }
    }

    pub async fn retrieve(&self, query: &str) -> Result<String, MemoryError> {
        let embedding = self.embedder.embed(query).await
            .map_err(|e| MemoryError::Embedding(e.to_string()))?;
        let results = self.db.search(embedding, self.top_k).await?;
        let fragments: Vec<&str> = results.iter()
            .filter_map(|r| r.payload.get("text").and_then(|v| v.as_str()))
            .collect();
        Ok(fragments.join("\n\n"))
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
        fn name(&self) -> &str { "mock-embed" }
    }

    struct MockVectorDB;
    #[async_trait]
    impl VectorDB for MockVectorDB {
        async fn upsert(&self, _id: &str, _vector: Vec<f32>, _payload: serde_json::Value)
            -> Result<(), MemoryError> { Ok(()) }
        async fn search(&self, _query: Vec<f32>, _limit: usize)
            -> Result<Vec<SearchResult>, MemoryError>
        {
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
        assert!(result.contains("fragment_alpha"), "result should contain fragment_alpha");
        assert!(result.contains("fragment_beta"), "result should contain fragment_beta");
    }
}
