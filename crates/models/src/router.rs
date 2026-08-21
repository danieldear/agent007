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

    fn normalize_request_model(&self, selected_provider: &str, requested_model: &str) -> String {
        let requested_model = requested_model.trim();
        if requested_model.is_empty() {
            return String::new();
        }

        if let Some(stripped) = requested_model.strip_prefix(&format!("{selected_provider}/")) {
            if !stripped.is_empty() {
                return stripped.to_string();
            }
        }

        let resolved_requested = self.resolve_provider_name(requested_model);
        if self.providers.contains_key(resolved_requested)
            && resolved_requested == selected_provider
        {
            return requested_model.to_string();
        }

        selected_provider.to_string()
    }

    fn provider_key_for_task_type(&self, task_type: &str) -> String {
        if let Some(name) = self.rules.get(task_type) {
            self.resolve_provider_name(name).to_string()
        } else {
            self.default.clone()
        }
    }

    /// Route a task type to the appropriate provider.
    ///
    /// The default provider **must** be registered via `register()` before calling this method.
    /// Panics if neither the rule-matched provider nor the default provider is registered.
    #[instrument(skip(self), fields(provider = tracing::field::Empty))]
    pub fn route(&self, task_type: &str) -> Arc<dyn ModelProvider> {
        let provider_name = self.provider_key_for_task_type(task_type);

        // Look up provider, fall back to default if not found
        let provider = self
            .providers
            .get(provider_name.as_str())
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

    pub async fn complete_for_task_type(
        &self,
        task_type: &str,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ModelError> {
        let requested_model = request.model.clone();
        let key = self.provider_key_for_task_type(task_type);
        let provider = self.providers.get(&key).ok_or_else(|| {
            ModelError::NotConfigured(format!("no provider registered for '{key}'"))
        })?;
        let mut routed_request = request;
        routed_request.model = self.normalize_request_model(&key, &requested_model);
        provider.complete(routed_request).await
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
        let requested_model = request.model.clone();
        let requested = self.resolve_provider_name(&requested_model).to_string();
        let key = if self.providers.contains_key(&requested) {
            requested
        } else if let Some(rule) = self.rules.get(&request.model) {
            self.resolve_provider_name(rule).to_string()
        } else {
            self.default.clone()
        };
        let provider = self.providers.get(&key).ok_or_else(|| {
            ModelError::NotConfigured(format!("no provider registered for '{key}'"))
        })?;
        let mut routed_request = request;
        routed_request.model = self.normalize_request_model(&key, &requested_model);
        provider.complete(routed_request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockProvider;
    use crate::types::{CompletionRequest, CompletionResponse, Message, Role};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct EchoModelProvider {
        name: &'static str,
    }

    #[async_trait]
    impl ModelProvider for EchoModelProvider {
        fn name(&self) -> &str {
            self.name
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, ModelError> {
            Ok(CompletionResponse {
                content: request.model.clone(),
                model: request.model,
                input_tokens: None,
                output_tokens: None,
                cached_tokens: None,
                cache_write_tokens: None,
                total_tokens: None,
                estimated_cost_usd: None,
            })
        }
    }

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
        let mut r = make_router();
        r.alias("claude-sonnet-5", "claude");
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

    #[tokio::test]
    async fn router_rewrites_unavailable_model_to_selected_provider_default() {
        let mut r = ModelRouter::new("ollama");
        r.register(
            "ollama",
            Arc::new(EchoModelProvider {
                name: "ollama/qwen2.5-coder:7b",
            }),
        );
        r.alias("claude-sonnet-5", "claude");

        let resp = r
            .complete(CompletionRequest {
                model: "claude-sonnet-5".into(),
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

        assert_eq!(resp.content, "ollama");
    }

    #[tokio::test]
    async fn router_strips_provider_prefix_from_explicit_model_hint() {
        let mut r = ModelRouter::new("ollama");
        r.register(
            "ollama",
            Arc::new(EchoModelProvider {
                name: "ollama/qwen2.5-coder:7b",
            }),
        );
        r.alias("ollama/qwen2.5-coder:7b", "ollama");

        let resp = r
            .complete(CompletionRequest {
                model: "ollama/qwen2.5-coder:7b".into(),
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

        assert_eq!(resp.content, "qwen2.5-coder:7b");
    }

    #[tokio::test]
    async fn router_complete_for_task_type_normalizes_foreign_model_hint() {
        let mut r = ModelRouter::new("ollama");
        r.register(
            "ollama",
            Arc::new(EchoModelProvider {
                name: "ollama/qwen2.5-coder:7b",
            }),
        );
        r.add_rule("reasoning", "ollama");
        r.alias("claude-sonnet-5", "claude");

        let resp = r
            .complete_for_task_type(
                "reasoning",
                CompletionRequest {
                    model: "claude-sonnet-5".into(),
                    messages: vec![Message {
                        role: Role::User,
                        content: "plan".into(),
                    }],
                    max_tokens: None,
                    temperature: None,
                    system: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.content, "ollama");
    }
}
