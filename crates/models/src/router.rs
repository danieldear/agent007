use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse};

pub struct ModelRouter {
    providers: HashMap<String, Arc<dyn ModelProvider>>,
    default: Option<String>,
}

impl ModelRouter {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            default: None,
        }
    }

    pub fn register(&mut self, name: impl Into<String>, provider: Arc<dyn ModelProvider>) {
        self.providers.insert(name.into(), provider);
    }

    pub fn set_default(&mut self, name: impl Into<String>) {
        self.default = Some(name.into());
    }
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ModelProvider for ModelRouter {
    fn name(&self) -> &str {
        "router"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        // Route by request.model first; fall back to default provider name.
        let key = if self.providers.contains_key(&request.model) {
            request.model.as_str()
        } else {
            self.default.as_deref().unwrap_or(&request.model)
        };
        let provider = self.providers.get(key).ok_or_else(|| {
            ModelError::NotConfigured(format!("no provider registered for '{key}'"))
        })?;
        provider.complete(request).await
    }
}
