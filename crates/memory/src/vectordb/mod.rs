pub mod lancedb;
pub use lancedb::LanceDBStore;

use async_trait::async_trait;
use crate::error::MemoryError;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub payload: serde_json::Value,
}

#[async_trait]
pub trait VectorDB: Send + Sync {
    async fn upsert(&self, id: &str, vector: Vec<f32>, payload: serde_json::Value)
        -> Result<(), MemoryError>;
    async fn search(&self, query: Vec<f32>, limit: usize)
        -> Result<Vec<SearchResult>, MemoryError>;
}
