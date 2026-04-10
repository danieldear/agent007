use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::instrument;

pub struct ModelRouter {
    providers: HashMap<String, Arc<dyn ModelProvider>>,
    rules: HashMap<String, String>,
    aliases: HashMap<String, String>,
    default: String,
}

impl ModelRouter {
    pub fn new(default_provider: &str) -> Self {
        Self {
            providers: HashMap::new(),
            rules: HashMap::new(),
            aliases: HashMap::new(),
            default: default_provider.to_string(),
        }
    }

    pub fn register(&mut self, name: &str, provider: Arc<dyn ModelProvider>) {
        self.providers.insert(name.to_string(), provider);
        self.aliases.insert(name.to_string(), name.to_string());
    }

    pub fn add_rule(&mut self, task_type: &str, provider_name: &str) {
        self.rules
            .insert(task_type.to_string(), provider_name.to_string());
    }

    pub fn alias(&mut self, alias: &str, provider_name: &str) {
        self.aliases
            .insert(alias.to_string(), provider_name.to_string());
    }

    fn resolve_provider_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.aliases.get(key).map(String::as_str).unwrap_or(key)
    }

    /// Route a task type to the appropriate provider.
    ///
    /// The default provider **must** be registered via `register()` before calling this method.
    /// Panics if neither the rule-matched provider nor the default provider is registered.
    #[instrument(skip(self), fields(provider = tracing::field::Empty))]
    pub fn route(&self, task_type: &str) -> Arc<dyn ModelProvider> {
        // Check rules first
        let provider_name = if let Some(name) = self.rules.get(task_type) {
            self.resolve_provider_name(name)
        } else {
            self.default.as_str()
        };

        // Look up provider, fall back to default if not found
        let provider = self
            .providers
            .get(provider_name)
            .cloned()
            .unwrap_or_else(|| {
                self.providers
                    .get(self.default.as_str())
                    .cloned()
                    .unwrap_or_else(|| {
                        panic!(
                        "default provider '{}' is not registered — call register() before route()",
                        self.default
                    )
                    })
            });
        tracing::Span::current().record("provider", provider.name());
        provider
    }
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::new("default")
    }
}

#[async_trait]
impl ModelProvider for ModelRouter {
    fn name(&self) -> &str {
        "router"
    }

    #[instrument(skip(self, request), fields(model = %request.model))]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let requested = self.resolve_provider_name(&request.model);
        let key = if self.providers.contains_key(requested) {
            requested
        } else if let Some(rule) = self.rules.get(&request.model) {
            self.resolve_provider_name(rule)
        } else {
            self.default.as_str()
        };
        let provider = self.providers.get(key).ok_or_else(|| {
            ModelError::NotConfigured(format!("no provider registered for '{key}'"))
        })?;
        provider.complete(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockProvider;
    use std::sync::Arc;

    fn make_router() -> ModelRouter {
        let mut r = ModelRouter::new("claude");
        r.register(
            "claude",
            Arc::new(MockProvider::new("claude-resp", "claude")),
        );
        r.register("codex", Arc::new(MockProvider::new("codex-resp", "codex")));
        r.register(
            "ollama",
            Arc::new(MockProvider::new("ollama-resp", "ollama")),
        );
        r
    }

    #[test]
    fn router_falls_back_to_default() {
        let r = make_router();
        assert_eq!(r.route("unknown").name(), "claude");
    }

    #[test]
    fn router_picks_rule_over_default() {
        let mut r = make_router();
        r.add_rule("code_completion", "codex");
        assert_eq!(r.route("code_completion").name(), "codex");
        assert_eq!(r.route("reasoning").name(), "claude"); // no rule → default
    }

    #[tokio::test]
    async fn router_routes_to_correct_provider_output() {
        use crate::types::{CompletionRequest, Message, Role};
        let mut r = make_router();
        r.add_rule("code_completion", "codex");
        let resp = r
            .route("code_completion")
            .complete(CompletionRequest {
                model: "any".into(),
                messages: vec![Message {
                    role: Role::User,
                    content: "write code".into(),
                }],
                max_tokens: None,
                temperature: None,
                system: None,
            })
            .await
            .unwrap();
        assert_eq!(resp.content, "codex-resp");
    }

    #[tokio::test]
    async fn router_resolves_model_aliases() {
        use crate::types::{CompletionRequest, Message, Role};
        let mut r = make_router();
        r.alias("claude-sonnet-4-6", "claude");
        r.alias("gpt-5.3-codex", "codex");

        let resp = r
            .complete(CompletionRequest {
                model: "gpt-5.3-codex".into(),
                messages: vec![Message {
                    role: Role::User,
                    content: "write code".into(),
                }],
                max_tokens: None,
                temperature: None,
                system: None,
            })
            .await
            .unwrap();

        assert_eq!(resp.content, "codex-resp");
    }
}
