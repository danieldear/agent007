use crate::error::ModelError;
use crate::types::{CompletionRequest, CompletionResponse};
use async_trait::async_trait;

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError>;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ModelError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    struct AlwaysHelloProvider;

    #[async_trait]
    impl ModelProvider for AlwaysHelloProvider {
        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ModelError> {
            Ok(CompletionResponse {
                content: "hello".to_string(),
                model: "test".to_string(),
                input_tokens: None,
                output_tokens: None,
                cached_tokens: None,
            })
        }
        fn name(&self) -> &str {
            "test"
        }
    }

    #[tokio::test]
    async fn provider_is_object_safe_and_callable() {
        let provider: Box<dyn ModelProvider> = Box::new(AlwaysHelloProvider);
        let req = CompletionRequest {
            model: "test".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "hi".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            system: None,
        };
        let resp = provider.complete(req).await.unwrap();
        assert_eq!(resp.content, "hello");
        assert_eq!(provider.name(), "test");
    }

    #[tokio::test]
    async fn embedding_provider_is_object_safe() {
        struct ZeroEmbedder;
        #[async_trait]
        impl EmbeddingProvider for ZeroEmbedder {
            async fn embed(&self, _text: &str) -> Result<Vec<f32>, ModelError> {
                Ok(vec![0.0; 4])
            }
            fn name(&self) -> &str {
                "zero"
            }
        }
        let ep: Box<dyn EmbeddingProvider> = Box::new(ZeroEmbedder);
        let v = ep.embed("test").await.unwrap();
        assert_eq!(v.len(), 4);
    }
}
