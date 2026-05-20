pub mod lancedb;
pub use lancedb::LanceDBStore;

use crate::error::MemoryError;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub payload: serde_json::Value,
}

#[async_trait]
pub trait VectorDB: Send + Sync {
    async fn upsert(
        &self,
        id: &str,
        vector: Vec<f32>,
        payload: serde_json::Value,
    ) -> Result<(), MemoryError>;
    async fn search(&self, query: Vec<f32>, limit: usize)
        -> Result<Vec<SearchResult>, MemoryError>;
    /// Delete all chunks whose id equals `doc_id` or starts with `doc_id#`.
    /// Called before re-indexing an updated document so stale higher-index
    /// chunks from a previous longer value do not remain searchable.
    /// Default implementation is a no-op (safe for mocks and no-op DBs).
    async fn delete_doc(&self, _doc_id: &str) -> Result<(), MemoryError> {
        Ok(())
    }
}
