use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

// ── data model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRegistryEntry {
    pub name: String,
    pub source_kind: String, // "npm" | "local" | "github" | "http" | "manual"
    pub source_ref: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
    #[serde(default = "default_status")]
    pub status: String, // "disconnected" | "connecting" | "connected" | "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_msg: Option<String>,
    #[serde(default = "default_scope")]
    pub scope: String, // "project" | "global"
    pub added_at: String, // RFC3339
}

fn default_status() -> String {
    "disconnected".to_string()
}

fn default_scope() -> String {
    "project".to_string()
}

// ── storage ───────────────────────────────────────────────────────────────────

fn registry_path(project_home: &Path) -> PathBuf {
    project_home.join("mcp").join("registry.json")
}

pub fn load_mcp_registry(project_home: &Path) -> Result<Vec<McpRegistryEntry>, String> {
    let path = registry_path(project_home);
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str::<Vec<McpRegistryEntry>>(&raw)
        .map_err(|e| format!("invalid {}: {e}", path.display()))
}

pub fn save_mcp_registry(project_home: &Path, entries: &[McpRegistryEntry]) -> Result<(), String> {
    let path = registry_path(project_home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

// ── operations ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddMcpServerRequest {
    pub name: Option<String>,
    pub source_kind: String,
    pub source_ref: String,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_scope")]
    pub scope: String,
}

pub fn add_mcp_server(
    project_home: &Path,
    req: AddMcpServerRequest,
) -> Result<McpRegistryEntry, String> {
    let mut entries = load_mcp_registry(project_home)?;

    // derive command + args from source_kind when not provided
    let (command, args) = match req.source_kind.as_str() {
        "npm" => (
            req.command.unwrap_or_else(|| "npx".to_string()),
            if req.args.is_empty() {
                vec!["-y".to_string(), req.source_ref.clone()]
            } else {
                req.args.clone()
            },
        ),
        _ => (
            req.command
                .ok_or_else(|| "command is required for non-npm sources".to_string())?,
            req.args.clone(),
        ),
    };

    let name = req.name.unwrap_or_else(|| {
        // derive a slug from source_ref
        req.source_ref
            .split('/')
            .last()
            .unwrap_or(&req.source_ref)
            .trim_start_matches('@')
            .replace(['.', '@', '/'], "-")
            .to_string()
    });

    if entries.iter().any(|e| e.name == name) {
        return Err(format!("server '{}' already exists in registry", name));
    }

    let entry = McpRegistryEntry {
        name,
        source_kind: req.source_kind,
        source_ref: req.source_ref,
        command,
        args,
        env: req.env,
        approved: false,
        tools: vec![],
        status: "disconnected".to_string(),
        error_msg: None,
        scope: req.scope,
        added_at: chrono::Utc::now().to_rfc3339(),
    };

    entries.push(entry.clone());
    save_mcp_registry(project_home, &entries)?;
    Ok(entry)
}

pub fn delete_mcp_server(project_home: &Path, name: &str) -> Result<(), String> {
    let mut entries = load_mcp_registry(project_home)?;
    let before = entries.len();
    entries.retain(|e| e.name != name);
    if entries.len() == before {
        return Err(format!("server '{}' not found", name));
    }
    save_mcp_registry(project_home, &entries)
}

pub fn approve_mcp_server(project_home: &Path, name: &str) -> Result<McpRegistryEntry, String> {
    let mut entries = load_mcp_registry(project_home)?;
    let entry = entries
        .iter_mut()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("server '{}' not found", name))?;
    entry.approved = true;
    let result = entry.clone();
    save_mcp_registry(project_home, &entries)?;
    Ok(result)
}

pub fn get_mcp_server_tools(
    project_home: &Path,
    name: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let entries = load_mcp_registry(project_home)?;
    let entry = entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("server '{}' not found", name))?;
    Ok(entry.tools.clone())
}

/// Connect to an MCP server, discover its tools, and persist the result.
/// Returns the updated entry.
pub async fn connect_mcp_server(
    project_home: &Path,
    name: &str,
) -> Result<McpRegistryEntry, String> {
    // Mark as "connecting" first
    {
        let mut entries = load_mcp_registry(project_home)?;
        if let Some(e) = entries.iter_mut().find(|e| e.name == name) {
            e.status = "connecting".to_string();
            e.error_msg = None;
        }
        save_mcp_registry(project_home, &entries)?;
    }

    let entries = load_mcp_registry(project_home)?;
    let entry = entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("server '{}' not found", name))?
        .clone();

    let config = agent007_mcp::McpServerConfig {
        name: entry.name.clone(),
        command: entry.command.clone(),
        args: entry.args.clone(),
        env: entry.env.clone(),
        cwd: None,
    };

    let mut client = agent007_mcp::McpClient::new(vec![config]);

    let (discovered_tools, connect_error) = match client.connect().await {
        Ok(()) => {
            let tools_json: Vec<serde_json::Value> = client
                .list_tools()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    })
                })
                .collect();
            (tools_json, None)
        }
        Err(e) => (vec![], Some(e.to_string())),
    };

    // Update the registry with discovered tools and final status
    let mut entries = load_mcp_registry(project_home)?;
    let target = entries
        .iter_mut()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("server '{}' not found after connect", name))?;

    if connect_error.is_some() {
        target.status = "error".to_string();
        target.error_msg = connect_error;
    } else {
        target.status = "connected".to_string();
        target.error_msg = None;
        target.tools = discovered_tools;
    }

    let result = target.clone();
    save_mcp_registry(project_home, &entries)?;
    Ok(result)
}

pub async fn refresh_mcp_server_statuses(
    project_home: &Path,
) -> Result<Vec<McpRegistryEntry>, String> {
    let mut entries = load_mcp_registry(project_home)?;
    let mut changed = false;

    for entry in &mut entries {
        if entry.status != "connected" {
            continue;
        }
        let (alive, tools_or_err) = probe_mcp_server(entry).await;
        if alive {
            if let Ok(tools) = tools_or_err {
                if !tools.is_empty() && tools != entry.tools {
                    entry.tools = tools;
                    changed = true;
                }
                if entry.error_msg.is_some() {
                    entry.error_msg = None;
                    changed = true;
                }
            }
            continue;
        }
        entry.status = "error".to_string();
        entry.error_msg = Some(
            tools_or_err
                .err()
                .unwrap_or_else(|| "MCP server probe failed".to_string()),
        );
        changed = true;
    }

    if changed {
        save_mcp_registry(project_home, &entries)?;
    }
    Ok(entries)
}

async fn probe_mcp_server(
    entry: &McpRegistryEntry,
) -> (bool, Result<Vec<serde_json::Value>, String>) {
    let config = agent007_mcp::McpServerConfig {
        name: entry.name.clone(),
        command: entry.command.clone(),
        args: entry.args.clone(),
        env: entry.env.clone(),
        cwd: None,
    };
    let mut client = agent007_mcp::McpClient::new(vec![config]);
    match client.connect().await {
        Ok(()) => {
            let tools = client
                .list_tools()
                .await
                .map_err(|e| e.to_string())
                .unwrap_or_default()
                .into_iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    })
                })
                .collect();
            (true, Ok(tools))
        }
        Err(e) => (false, Err(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{load_mcp_registry, save_mcp_registry, McpRegistryEntry};

    #[test]
    fn load_registry_reports_invalid_json() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("mcp");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("registry.json"), "{not-json").unwrap();
        let err = load_mcp_registry(tmp.path()).unwrap_err();
        assert!(err.contains("invalid"));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = vec![McpRegistryEntry {
            name: "demo".to_string(),
            source_kind: "npm".to_string(),
            source_ref: "@demo/server".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@demo/server".to_string()],
            env: std::collections::HashMap::new(),
            approved: false,
            tools: vec![],
            status: "disconnected".to_string(),
            error_msg: None,
            scope: "project".to_string(),
            added_at: "2026-01-01T00:00:00Z".to_string(),
        }];
        save_mcp_registry(tmp.path(), &entries).unwrap();
        let loaded = load_mcp_registry(tmp.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "demo");
    }
}
