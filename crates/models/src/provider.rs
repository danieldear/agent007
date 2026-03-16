use async_trait::async_trait;
use crate::error::ModelError;
use crate::types::{CompletionRequest, CompletionResponse};

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError>;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ModelError>;
}
