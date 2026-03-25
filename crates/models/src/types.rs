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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_request_serializes() {
        let req = CompletionRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![Message { role: Role::User, content: "hello".to_string() }],
            max_tokens: Some(100),
            temperature: None,
            system: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("claude-sonnet-4-6"));
        assert!(json.contains("hello"));
        assert!(!json.contains("temperature")); // None fields skipped
    }

    #[test]
    fn completion_response_roundtrips() {
        let resp = CompletionResponse {
            content: "world".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            input_tokens: Some(5),
            output_tokens: Some(1),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: CompletionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "world");
        assert_eq!(back.input_tokens, Some(5));
    }

    #[test]
    fn role_serializes_lowercase() {
        let json = serde_json::to_string(&Role::User).unwrap();
        assert_eq!(json, "\"user\"");
    }
}
