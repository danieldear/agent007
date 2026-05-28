use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse, Message, Role};
use async_trait::async_trait;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy)]
struct ClaudePricing {
    input_per_mtok: f64,
    cache_read_per_mtok: f64,
    cache_write_per_mtok: f64,
    output_per_mtok: f64,
}

fn claude_pricing_for_model(model: &str) -> Option<ClaudePricing> {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.contains("haiku-3-5") || normalized.contains("haiku 3.5") {
        return Some(ClaudePricing {
            input_per_mtok: 0.80,
            cache_read_per_mtok: 0.08,
            cache_write_per_mtok: 1.0,
            output_per_mtok: 4.0,
        });
    }
    if normalized.contains("haiku") {
        return Some(ClaudePricing {
            input_per_mtok: 0.25,
            cache_read_per_mtok: 0.03,
            cache_write_per_mtok: 0.30,
            output_per_mtok: 1.25,
        });
    }
    if normalized.contains("opus") {
        return Some(ClaudePricing {
            input_per_mtok: 15.0,
            cache_read_per_mtok: 1.50,
            cache_write_per_mtok: 18.75,
            output_per_mtok: 75.0,
        });
    }
    if normalized.contains("sonnet") {
        return Some(ClaudePricing {
            input_per_mtok: 3.0,
            cache_read_per_mtok: 0.30,
            cache_write_per_mtok: 3.75,
            output_per_mtok: 15.0,
        });
    }
    None
}

fn claude_estimated_cost_usd(
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
) -> Option<f64> {
    let pricing = claude_pricing_for_model(model)?;
    let input_cost = input_tokens as f64 * pricing.input_per_mtok / 1_000_000.0;
    let output_cost = output_tokens as f64 * pricing.output_per_mtok / 1_000_000.0;
    let cache_read_cost = cache_read_tokens as f64 * pricing.cache_read_per_mtok / 1_000_000.0;
    let cache_write_cost = cache_write_tokens as f64 * pricing.cache_write_per_mtok / 1_000_000.0;
    Some(input_cost + output_cost + cache_read_cost + cache_write_cost)
}

pub struct ClaudeProvider {
    api_key: String,
    model: String,
}

impl ClaudeProvider {
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
        // Filter out system messages from the messages array
        let filtered_messages: Vec<_> = messages
            .iter()
            .filter(|msg| msg.role != Role::System)
            .cloned()
            .collect();

        let mut body = json!({
            "model": model,
            "messages": filtered_messages,
            "max_tokens": max_tokens.unwrap_or(4096),
        });

        if let Some(temperature) = temperature {
            body["temperature"] = json!(temperature);
        }

        // Use prompt caching for long system prompts (>1000 chars reduces cost on repeat calls).
        let caching = if let Some(system_content) = system {
            if system_content.len() > 1000 {
                body["system"] = json!([{
                    "type": "text",
                    "text": system_content,
                    "cache_control": {"type": "ephemeral"}
                }]);
                true
            } else {
                body["system"] = json!(system_content);
                false
            }
        } else {
            false
        };
        body["_caching"] = json!(caching);

        body.to_string()
    }
}

#[async_trait]
impl ModelProvider for ClaudeProvider {
    fn name(&self) -> &str {
        "claude"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let model = if request.model.is_empty() || request.model == self.name() {
            self.model.as_str()
        } else {
            request.model.as_str()
        };
        let body_str = self.build_body(
            model,
            &request.messages,
            request.max_tokens,
            request.temperature,
            request.system.as_deref(),
        );
        // Parse to extract and strip the internal _caching flag before sending.
        let mut body_val: Value = serde_json::from_str(&body_str)
            .expect("build_body produced invalid JSON — this is a bug");
        let caching_active = body_val
            .get("_caching")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        body_val.as_object_mut().map(|m| m.remove("_caching"));
        let body = body_val.to_string();

        let client = reqwest::Client::new();
        let url = "https://api.anthropic.com/v1/messages";

        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01");
        if caching_active {
            req = req.header("anthropic-beta", "prompt-caching-2024-07-31");
        }
        let response = req.body(body).send().await?;

        if !response.status().is_success() {
            return Err(ModelError::Api {
                provider: self.name().to_string(),
                message: format!("HTTP {}", response.status()),
            });
        }

        let json: Value = response.json().await?;

        let content = json["content"][0]["text"]
            .as_str()
            .ok_or_else(|| ModelError::Api {
                provider: self.name().to_string(),
                message: "missing or invalid content field".to_string(),
            })?
            .to_string();

        let input_tokens = json["usage"]["input_tokens"].as_u64().map(|x| x as u32);
        let output_tokens = json["usage"]["output_tokens"].as_u64().map(|x| x as u32);
        let cache_read_tokens = json["usage"]["cache_read_input_tokens"]
            .as_u64()
            .map(|x| x as u32);
        let cache_write_tokens = json["usage"]["cache_creation_input_tokens"]
            .as_u64()
            .map(|x| x as u32);
        let total_tokens = Some(
            input_tokens.unwrap_or(0) as u64
                + output_tokens.unwrap_or(0) as u64
                + cache_read_tokens.unwrap_or(0) as u64
                + cache_write_tokens.unwrap_or(0) as u64,
        );
        let estimated_cost_usd = match (
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        ) {
            (Some(input), Some(output), read, write) => claude_estimated_cost_usd(
                model,
                input,
                output,
                read.unwrap_or(0),
                write.unwrap_or(0),
            ),
            _ => None,
        };

        Ok(CompletionResponse {
            content,
            model: model.to_string(),
            input_tokens,
            output_tokens,
            cached_tokens: cache_read_tokens,
            cache_write_tokens,
            total_tokens,
            estimated_cost_usd,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_provider_name() {
        let p = ClaudeProvider::new("key", "claude-sonnet-4-6");
        assert_eq!(p.name(), "claude");
    }

    #[test]
    fn claude_builds_correct_request_body() {
        let p = ClaudeProvider::new("key", "claude-sonnet-4-6");
        let msgs = vec![Message {
            role: Role::User,
            content: "hi".to_string(),
        }];
        let body = p.build_body("claude-sonnet-4-6", &msgs, Some(100), None, None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "claude-sonnet-4-6");
        assert_eq!(v["max_tokens"], 100);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hi");
    }

    #[test]
    fn claude_prompt_caching_enabled_for_long_system() {
        let p = ClaudeProvider::new("key", "claude-sonnet-4-6");
        let msgs = vec![Message {
            role: Role::User,
            content: "hello".to_string(),
        }];
        let long_system = "x".repeat(1001);
        let body = p.build_body("claude-sonnet-4-6", &msgs, None, None, Some(&long_system));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        // system should be an array with cache_control
        let system = &v["system"];
        assert!(
            system.is_array(),
            "system should be an array when content >1000 chars"
        );
        let block = &system[0];
        assert_eq!(block["type"], "text");
        assert_eq!(block["cache_control"]["type"], "ephemeral");
        assert!(v["_caching"].as_bool().unwrap_or(false));
    }

    #[test]
    fn claude_prompt_caching_disabled_for_short_system() {
        let p = ClaudeProvider::new("key", "claude-sonnet-4-6");
        let msgs = vec![Message {
            role: Role::User,
            content: "hello".to_string(),
        }];
        let body = p.build_body("claude-sonnet-4-6", &msgs, None, None, Some("short system"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        // system should be a plain string when content ≤1000 chars
        assert!(
            v["system"].is_string(),
            "system should be plain string when content ≤1000 chars"
        );
        assert!(!v["_caching"].as_bool().unwrap_or(true));
    }

    #[test]
    fn claude_filters_system_messages_from_messages_array() {
        let p = ClaudeProvider::new("key", "claude-sonnet-4-6");
        let msgs = vec![
            Message {
                role: Role::System,
                content: "you are helpful".to_string(),
            },
            Message {
                role: Role::User,
                content: "hello".to_string(),
            },
        ];
        let body = p.build_body("claude-sonnet-4-6", &msgs, None, None, None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        // Claude system is a top-level field, not in messages array
        assert_eq!(v["messages"].as_array().unwrap().len(), 1);
        assert_eq!(v["messages"][0]["role"], "user");
    }

    #[test]
    fn claude_cost_estimate_handles_cache_read_and_write() {
        let estimate = claude_estimated_cost_usd("claude-sonnet-4-6", 1000, 200, 3000, 4000)
            .expect("known sonnet pricing");
        let expected = (1000.0 * 3.0 + 200.0 * 15.0 + 3000.0 * 0.30 + 4000.0 * 3.75) / 1_000_000.0;
        assert!((estimate - expected).abs() < f64::EPSILON);
    }
}
