use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse, Message, Role};
use async_trait::async_trait;
use serde_json::{json, Value};

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

    pub fn build_body(
        &self,
        model: &str,
        messages: &[Message],
        max_tokens: Option<u32>,
        temperature: Option<f32>,
        system: Option<&str>,
    ) -> String {
        let mut input = Vec::new();
        if let Some(system) = system {
            input.push(json!({
                "role": "system",
                "content": [{"type": "input_text", "text": system}],
            }));
        }
        for message in messages {
            let role = match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            input.push(json!({
                "role": role,
                "content": [{"type": "input_text", "text": message.content}],
            }));
        }

        let mut body = json!({
            "model": model,
            "input": input,
        });

        if let Some(max_tokens) = max_tokens {
            body["max_output_tokens"] = json!(max_tokens);
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
        let model = if request.model.is_empty() || request.model == self.name() {
            self.model.as_str()
        } else {
            request.model.as_str()
        };
        let body = self.build_body(
            model,
            &request.messages,
            request.max_tokens,
            request.temperature,
            request.system.as_deref(),
        );

        let client = reqwest::Client::new();
        let url = "https://api.openai.com/v1/responses";

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

        let content = extract_output_text(&json).ok_or_else(|| ModelError::Api {
            provider: self.name().to_string(),
            message: "missing or invalid content field".to_string(),
        })?;

        Ok(CompletionResponse {
            content,
            model: model.to_string(),
            input_tokens: json["usage"]["input_tokens"].as_u64().map(|x| x as u32),
            output_tokens: json["usage"]["output_tokens"].as_u64().map(|x| x as u32),
            cached_tokens: json["usage"]["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .map(|x| x as u32),
        })
    }
}

fn extract_output_text(response: &Value) -> Option<String> {
    if let Some(text) = response.get("output_text").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    let output = response.get("output")?.as_array()?;
    let mut parts = Vec::new();
    for item in output {
        let contents = item.get("content").and_then(Value::as_array);
        if let Some(contents) = contents {
            for content in contents {
                if let Some(text) = content.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    #[test]
    fn codex_provider_name() {
        let p = CodexProvider::new("key", "gpt-5.3-codex");
        assert_eq!(p.name(), "codex");
    }

    #[test]
    fn codex_builds_openai_body() {
        let p = CodexProvider::new("key", "gpt-5.3-codex");
        let msgs = vec![Message {
            role: Role::User,
            content: "hi".to_string(),
        }];
        let body = p.build_body("gpt-5.3-codex", &msgs, Some(50), None, Some("be precise"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "gpt-5.3-codex");
        assert_eq!(v["max_output_tokens"], 50);
        assert_eq!(v["input"][0]["role"], "system");
        assert_eq!(v["input"][1]["role"], "user");
        assert_eq!(v["input"][1]["content"][0]["text"], "hi");
    }

    #[test]
    fn codex_extracts_cached_tokens_from_usage() {
        let response = serde_json::json!({
            "output_text": "hello",
            "usage": {
                "input_tokens": 200,
                "output_tokens": 50,
                "input_tokens_details": {
                    "cached_tokens": 150
                }
            }
        });
        // Verify the path we parse in complete()
        let cached = response["usage"]["input_tokens_details"]["cached_tokens"]
            .as_u64()
            .map(|x| x as u32);
        assert_eq!(cached, Some(150));

        // Also verify no cached tokens when field absent
        let no_cache = serde_json::json!({"output_text": "hi", "usage": {"input_tokens": 10}});
        let cached_none = no_cache["usage"]["input_tokens_details"]["cached_tokens"]
            .as_u64()
            .map(|x| x as u32);
        assert_eq!(cached_none, None);
    }

    #[test]
    fn codex_extracts_output_text_from_responses_api_shape() {
        let response = serde_json::json!({
            "output": [{
                "content": [
                    {"type": "output_text", "text": "hello"},
                    {"type": "output_text", "text": "world"}
                ]
            }]
        });
        assert_eq!(
            extract_output_text(&response).as_deref(),
            Some("hello\nworld")
        );
    }
}
