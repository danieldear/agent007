use async_trait::async_trait;
use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse};

pub struct MockProvider {
    pub response: String,
}

impl MockProvider {
    pub fn new(response: impl Into<String>) -> Self {
        Self { response: response.into() }
    }
}

#[async_trait]
impl ModelProvider for MockProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        Ok(CompletionResponse {
            content: self.response.clone(),
            model: request.model,
        })
    }
}
