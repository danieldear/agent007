use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    /// Tokens served from provider cache (prompt caching hit). None if not reported or not cached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
    /// Tokens written into provider-side prompt cache on this request, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
    /// Provider-normalized total tokens for this request. This avoids double-counting cache
    /// read tokens on APIs where cached tokens are already included in input_tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// Provider-specific cost estimate in USD when the provider can calculate it precisely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
}

impl CompletionResponse {
    pub fn total_tokens_with_fallback(&self) -> Option<u64> {
        self.total_tokens.or_else(|| {
            self.input_tokens
                .zip(self.output_tokens)
                .map(|(input, output)| (input as u64) + (output as u64))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_request_serializes() {
        let req = CompletionRequest {
            model: "claude-sonnet-5".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "hello".to_string(),
            }],
            max_tokens: Some(100),
            temperature: None,
            system: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("claude-sonnet-5"));
        assert!(json.contains("hello"));
        assert!(!json.contains("temperature")); // None fields skipped
    }

    #[test]
    fn completion_response_roundtrips() {
        let resp = CompletionResponse {
            content: "world".to_string(),
            model: "claude-sonnet-5".to_string(),
            input_tokens: Some(5),
            output_tokens: Some(1),
            cached_tokens: Some(3),
            cache_write_tokens: Some(2),
            total_tokens: Some(11),
            estimated_cost_usd: Some(0.000123),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: CompletionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "world");
        assert_eq!(back.input_tokens, Some(5));
        assert_eq!(back.cached_tokens, Some(3));
        assert_eq!(back.cache_write_tokens, Some(2));
        assert_eq!(back.total_tokens, Some(11));
        assert_eq!(back.estimated_cost_usd, Some(0.000123));
    }

    #[test]
    fn role_serializes_lowercase() {
        let json = serde_json::to_string(&Role::User).unwrap();
        assert_eq!(json, "\"user\"");
    }
}
