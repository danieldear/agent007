use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse, Message};
use async_trait::async_trait;
use serde_json::{json, Value};

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

    pub fn build_body(
        &self,
        model: &str,
        messages: &[Message],
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> String {
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
        let model = if request.model.is_empty()
            || request.model == self.name()
            || request.model == "ollama"
        {
            self.model.as_str()
        } else {
            request.model.as_str()
        };

        // Prepend system message if provided
        if let Some(system_content) = request.system {
            messages.insert(
                0,
                Message {
                    role: crate::types::Role::System,
                    content: system_content,
                },
            );
        }

        let body = self.build_body(model, &messages, request.max_tokens, request.temperature);

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
            model: model.to_string(),
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
        let msgs = vec![Message {
            role: Role::User,
            content: "hello".to_string(),
        }];
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

use crate::provider::EmbeddingProvider;

pub struct OllamaEmbeddingProvider {
    base_url: String,
    model: String,
}

impl OllamaEmbeddingProvider {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            model: model.to_string(),
        }
    }

    async fn post_json(&self, path: &str, body: Value) -> Result<Value, ModelError> {
        let client = reqwest::Client::new();
        let url = format!("{}{}", self.base_url, path);

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ModelError::Api {
                provider: "ollama-embed".to_string(),
                message: format!("HTTP {}", response.status()),
            });
        }

        Ok(response.json().await?)
    }

    fn parse_embedding(json: &Value) -> Result<Vec<f32>, ModelError> {
        if let Some(embeddings) = json.get("embeddings").and_then(Value::as_array) {
            if let Some(first) = embeddings.first().and_then(Value::as_array) {
                return Ok(first
                    .iter()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                    .collect());
            }
        }

        if let Some(embedding) = json.get("embedding").and_then(Value::as_array) {
            return Ok(embedding
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect());
        }

        Err(ModelError::Api {
            provider: "ollama-embed".to_string(),
            message: "missing embedding field".to_string(),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbeddingProvider {
    fn name(&self) -> &str {
        "ollama-embed"
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, ModelError> {
        match self
            .post_json(
                "/api/embed",
                serde_json::json!({
                    "model": self.model,
                    "input": text,
                }),
            )
            .await
        {
            Ok(json) => Self::parse_embedding(&json),
            Err(ModelError::Api { message, .. }) if message == "HTTP 404" => {
                let legacy = self
                    .post_json(
                        "/api/embeddings",
                        serde_json::json!({
                            "model": self.model,
                            "prompt": text,
                        }),
                    )
                    .await?;
                Self::parse_embedding(&legacy)
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod embedding_tests {
    use super::OllamaEmbeddingProvider;
    use serde_json::json;

    #[test]
    fn ollama_parse_current_embed_shape() {
        let parsed = OllamaEmbeddingProvider::parse_embedding(&json!({
            "embeddings": [[0.1, 0.2, 0.3]]
        }))
        .unwrap();
        assert_eq!(parsed, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn ollama_parse_legacy_embedding_shape() {
        let parsed = OllamaEmbeddingProvider::parse_embedding(&json!({
            "embedding": [0.4, 0.5]
        }))
        .unwrap();
        assert_eq!(parsed, vec![0.4, 0.5]);
    }
}
