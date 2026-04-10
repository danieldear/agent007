use crate::error::MemoryError;
use crate::vectordb::VectorDB;
use agent007_models::EmbeddingProvider;
use std::sync::Arc;

pub struct Indexer {
    embedder: Arc<dyn EmbeddingProvider>,
    db: Arc<dyn VectorDB>,
    chunk_size: usize,
}

impl Indexer {
    pub fn new(
        embedder: Arc<dyn EmbeddingProvider>,
        db: Arc<dyn VectorDB>,
        chunk_size: usize,
    ) -> Self {
        Self {
            embedder,
            db,
            chunk_size,
        }
    }

    pub async fn index_text(&self, doc_id: &str, text: &str) -> Result<(), MemoryError> {
        let chunks = self.chunk_text(text);
        for (n, chunk) in chunks.into_iter().enumerate() {
            let vector = self
                .embedder
                .embed(&chunk)
                .await
                .map_err(|e| MemoryError::Embedding(e.to_string()))?;
            let payload = serde_json::json!({
                "doc_id": doc_id,
                "chunk_index": n,
                "text": chunk,
            });
            let id = format!("{}#{}", doc_id, n);
            self.db.upsert(&id, vector, payload).await?;
        }
        Ok(())
    }

    fn chunk_text(&self, text: &str) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut current = String::new();
        for word in text.split_whitespace() {
            if !current.is_empty() && current.len() + 1 + word.len() > self.chunk_size {
                chunks.push(current.trim().to_string());
                current = word.to_string();
            } else {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            chunks.push(current.trim().to_string());
        }
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MemoryError;
    use crate::vectordb::{SearchResult, VectorDB};
    use agent007_models::{EmbeddingProvider, ModelError};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

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

    struct MockVectorDB {
        calls: Mutex<Vec<(String, Vec<f32>, serde_json::Value)>>,
    }
    impl MockVectorDB {
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
            }
        }
        fn upsert_calls(&self) -> Vec<(String, Vec<f32>, serde_json::Value)> {
            self.calls.lock().unwrap().clone()
        }
    }
    #[async_trait]
    impl VectorDB for MockVectorDB {
        async fn upsert(
            &self,
            id: &str,
            vector: Vec<f32>,
            payload: serde_json::Value,
        ) -> Result<(), MemoryError> {
            self.calls
                .lock()
                .unwrap()
                .push((id.to_string(), vector, payload));
            Ok(())
        }
        async fn search(
            &self,
            _query: Vec<f32>,
            _limit: usize,
        ) -> Result<Vec<SearchResult>, MemoryError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn indexer_chunks_and_upserts() {
        let embedder = Arc::new(MockEmbeddingProvider);
        let db = Arc::new(MockVectorDB::new());
        let indexer = Indexer::new(
            Arc::clone(&embedder) as Arc<dyn EmbeddingProvider>,
            Arc::clone(&db) as Arc<dyn VectorDB>,
            20, // chunk_size chars
        );

        indexer
            .index_text("doc1", "word1 word2 word3 word4 word5")
            .await
            .unwrap();

        let calls = db.upsert_calls();
        assert!(!calls.is_empty(), "should have upserted at least one chunk");

        // At least one upsert ID should start with "doc1#"
        assert!(
            calls.iter().any(|(id, _, _)| id.starts_with("doc1#")),
            "upsert IDs should start with doc1#"
        );

        // Payload should contain doc_id
        assert!(
            calls.iter().any(|(_, _, payload)| {
                payload.get("doc_id").and_then(|v| v.as_str()) == Some("doc1")
            }),
            "payload should contain doc_id: doc1"
        );
    }
}
