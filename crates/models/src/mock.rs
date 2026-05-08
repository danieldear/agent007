use crate::error::ModelError;
use crate::provider::{EmbeddingProvider, ModelProvider};
use crate::types::{CompletionRequest, CompletionResponse};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct MockProvider {
    response_content: String,
    model_name: String,
    call_count: Arc<AtomicUsize>,
    embedding_dim: usize,
}

impl MockProvider {
    pub fn new(response_content: &str, model_name: &str) -> Self {
        Self {
            response_content: response_content.to_string(),
            model_name: model_name.to_string(),
            call_count: Arc::new(AtomicUsize::new(0)),
            embedding_dim: 0,
        }
    }

    pub fn with_embedding_dim(response_content: &str, model_name: &str, dim: usize) -> Self {
        Self {
            response_content: response_content.to_string(),
            model_name: model_name.to_string(),
            call_count: Arc::new(AtomicUsize::new(0)),
            embedding_dim: dim,
        }
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ModelProvider for MockProvider {
    fn name(&self) -> &str {
        &self.model_name
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ModelError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(CompletionResponse {
            content: self.response_content.clone(),
            model: self.model_name.clone(),
            input_tokens: None,
            output_tokens: None,
            cached_tokens: None,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for MockProvider {
    fn name(&self) -> &str {
        &self.model_name
    }

    async fn embed(&self, _text: &str) -> Result<Vec<f32>, ModelError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(vec![0.0; self.embedding_dim])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompletionRequest, Message, Role};

    #[tokio::test]
    async fn mock_returns_configured_response() {
        let mock = MockProvider::new("mocked response", "mock-model");
        let req = CompletionRequest {
            model: "any".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "q".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            system: None,
        };
        let resp = mock.complete(req).await.unwrap();
        assert_eq!(resp.content, "mocked response");
        assert_eq!(resp.model, "mock-model");
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn mock_tracks_multiple_calls() {
        let mock = MockProvider::new("resp", "mock");
        let req = CompletionRequest {
            model: "any".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "q".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            system: None,
        };
        mock.complete(req.clone()).await.unwrap();
        mock.complete(req.clone()).await.unwrap();
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn mock_embedding_returns_zero_vector_of_given_dim() {
        let mock = MockProvider::with_embedding_dim("", "mock", 768);
        let v = mock.embed("hello").await.unwrap();
        assert_eq!(v.len(), 768);
        assert!(v.iter().all(|x| *x == 0.0));
    }
}
