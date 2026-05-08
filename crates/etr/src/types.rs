use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtrCallRequest {
    pub tool: String,
    pub input: serde_json::Value,
    #[serde(default = "default_true")]
    pub compact: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EtrCallResult {
    pub tool: String,
    pub status: EtrStatus,
    pub output: serde_json::Value,
    pub audit_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    pub latency_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EtrStatus {
    Ok,
    Error,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ToolLayer {
    L1,
    L2,
    L3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolManifest {
    pub name: String,
    pub layer: ToolLayer,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
}
