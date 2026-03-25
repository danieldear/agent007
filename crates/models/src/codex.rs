use async_trait::async_trait;
use serde_json::{json, Value};
use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse, Message};

pub struct CodexProvider {
    api_key: String,
    model: String,
}

impl CodexProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }

    pub fn build_body(&self, model: &str, messages: &[Message], max_tokens: Option<u32>, temperature: Option<f32>) -> String {
        let mut body = json!({
            "model": model,
            "messages": messages,
        });

        if let Some(max_tokens) = max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }

        if let Some(temperature) = temperature {
            body["temperature"] = json!(temperature);
        }

        body.to_string()
    }
}

#[async_trait]
impl ModelProvider for CodexProvider {
    fn name(&self) -> &str {
        "codex"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let body = self.build_body(
            &request.model,
            &request.messages,
            request.max_tokens,
            request.temperature,
        );

        let client = reqwest::Client::new();
        let url = "https://api.openai.com/v1/chat/completions";

        let response = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", &self.api_key))
            .body(body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ModelError::Api {
                provider: self.name().to_string(),
                message: format!("HTTP {}", response.status()),
            });
        }

        let json: Value = response.json().await?;

        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| ModelError::Api {
                provider: self.name().to_string(),
                message: "missing or invalid content field".to_string(),
            })?
            .to_string();

        Ok(CompletionResponse {
            content,
            model: request.model,
            input_tokens: json["usage"]["prompt_tokens"].as_u64().map(|x| x as u32),
            output_tokens: json["usage"]["completion_tokens"].as_u64().map(|x| x as u32),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    #[test]
    fn codex_provider_name() {
        let p = CodexProvider::new("key", "gpt-4o");
        assert_eq!(p.name(), "codex");
    }

    #[test]
    fn codex_builds_openai_body() {
        let p = CodexProvider::new("key", "gpt-4o");
        let msgs = vec![Message { role: Role::User, content: "hi".to_string() }];
        let body = p.build_body("gpt-4o", &msgs, Some(50), None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "gpt-4o");
        assert_eq!(v["max_tokens"], 50);
        assert_eq!(v["messages"][0]["role"], "user");
    }
}
