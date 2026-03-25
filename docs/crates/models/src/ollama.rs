use async_trait::async_trait;
use serde_json::{json, Value};
use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse, Message};

pub struct OllamaProvider {
    base_url: String,
    model: String,
    provider_name: String,
}

impl OllamaProvider {
    pub fn new(base_url: &str, model: &str) -> Self {
        let provider_name = format!("ollama/{}", model);
        Self {
            base_url: base_url.to_string(),
            model: model.to_string(),
            provider_name,
        }
    }

    pub fn build_body(&self, model: &str, messages: &[Message], max_tokens: Option<u32>, temperature: Option<f32>) -> String {
        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
        });

        if max_tokens.is_some() || temperature.is_some() {
            let mut options = json!({});
            if let Some(max_tokens) = max_tokens {
                options["num_predict"] = json!(max_tokens);
            }
            if let Some(temperature) = temperature {
                options["temperature"] = json!(temperature);
            }
            body["options"] = options;
        }

        body.to_string()
    }
}

#[async_trait]
impl ModelProvider for OllamaProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let mut messages = request.messages;

        // Prepend system message if provided
        if let Some(system_content) = request.system {
            messages.insert(0, Message {
                role: crate::types::Role::System,
                content: system_content,
            });
        }

        let body = self.build_body(&request.model, &messages, request.max_tokens, request.temperature);

        let client = reqwest::Client::new();
        let url = format!("{}/api/chat", self.base_url);

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
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

        let content = json["message"]["content"]
            .as_str()
            .ok_or_else(|| ModelError::Api {
                provider: self.name().to_string(),
                message: "missing or invalid content field".to_string(),
            })?
            .to_string();

        Ok(CompletionResponse {
            content,
            model: request.model,
            input_tokens: None,
            output_tokens: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Role;

    #[test]
    fn ollama_provider_name_includes_model() {
        let p = OllamaProvider::new("http://localhost:11434", "llama3");
        assert_eq!(p.name(), "ollama/llama3");
    }

    #[test]
    fn ollama_builds_openai_compatible_body() {
        let p = OllamaProvider::new("http://localhost:11434", "llama3");
        let msgs = vec![Message { role: Role::User, content: "hello".to_string() }];
        let body = p.build_body("llama3", &msgs, None, None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "llama3");
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hello");
        assert_eq!(v["stream"], false);
    }

    #[test]
    fn ollama_includes_options_when_set() {
        let p = OllamaProvider::new("http://localhost:11434", "llama3");
        let body = p.build_body("llama3", &[], Some(100), Some(0.5));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["options"]["num_predict"], 100);
        assert_eq!(v["options"]["temperature"], 0.5);
    }
}
