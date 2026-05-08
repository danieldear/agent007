use crate::mcp_registry::{add_mcp_server, delete_mcp_server, AddMcpServerRequest};
use crate::rag_sources::{
    add_rag_source, delete_rag_source, load_rag_sources, AddRagSourceRequest, RagKind,
};
use crate::server::AppState;
use agent007_core::paths::agent007_write_home;
use agent007_extensions::{
    adapters::{
        claude_marketplace::ClaudeMarketplaceAdapter, github::GitHubAdapter,
        mcp_npm::McpNpmAdapter, native::NativeAdapter, openapi::OpenApiAdapter,
    },
    bundle::BundleFile,
    ExtensionAdapter, ExtensionBundle, ExtensionSource,
};
use axum::http::StatusCode;
use axum::{extract::State, response::IntoResponse, Json};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

#[derive(Deserialize)]
pub struct PreviewRequest {
    pub source_kind: String,
    pub source_ref: String,
    pub github_owner: Option<String>,
    pub github_repo: Option<String>,
    pub github_ref: Option<String>,
}

fn build_source(req: &PreviewRequest) -> Result<ExtensionSource, String> {
    let source_kind = req.source_kind.trim().to_ascii_lowercase();
    match source_kind.as_str() {
        "local" => Ok(ExtensionSource::Local(std::path::PathBuf::from(
            &req.source_ref,
        ))),
        "github" => {
            let owner = req.github_owner.clone().unwrap_or_default();
            let repo = req.github_repo.clone().unwrap_or_default();
            if !owner.trim().is_empty() && !repo.trim().is_empty() {
                return Ok(ExtensionSource::GitHub {
                    owner,
                    repo,
                    ref_: req.github_ref.clone(),
                });
            }
            let fallback = req.source_ref.trim();
            if let Some((owner, repo)) = fallback.split_once('/') {
                Ok(ExtensionSource::GitHub {
                    owner: owner.to_string(),
                    repo: repo.to_string(),
                    ref_: req.github_ref.clone(),
                })
            } else {
                Err("github source requires owner/repo".to_string())
            }
        }
        "npm" | "mcp_npm" => Ok(ExtensionSource::McpNpm {
            package: req.source_ref.clone(),
        }),
        "openapi" | "url" => Ok(ExtensionSource::Url(req.source_ref.clone())),
        "claude" => Ok(ExtensionSource::Url(req.source_ref.clone())),
        other => Err(format!("unknown source_kind: {}", other)),
    }
}

async fn run_adapter(
    source_kind: &str,
    source: &ExtensionSource,
) -> Result<ExtensionBundle, String> {
    let source_kind = source_kind.trim().to_ascii_lowercase();
    let adapter: Box<dyn ExtensionAdapter> = match source_kind.as_str() {
        "local" => Box::new(NativeAdapter),
        "github" => Box::new(GitHubAdapter::new()),
        "npm" | "mcp_npm" => Box::new(McpNpmAdapter),
        "claude" => Box::new(ClaudeMarketplaceAdapter::new()),
        "openapi" | "url" => Box::new(OpenApiAdapter::new()),
        other => return Err(format!("no adapter available for source_kind '{other}'")),
    };
    if !adapter.can_handle(source) {
        return Err(format!(
            "adapter '{}' cannot handle source_kind '{}'",
            adapter.name(),
            source_kind
        ));
    }
    adapter.fetch(source).await.map_err(|e| e.to_string())
}

fn sanitize_relative_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty".to_string());
    }
    let rel = PathBuf::from(trimmed);
    if rel.is_absolute() {
        return Err(format!("absolute path is not allowed: '{}'", trimmed));
    }
    for comp in rel.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("parent traversal is not allowed: '{}'", trimmed))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("invalid path component in '{}'", trimmed))
            }
        }
    }
    Ok(rel)
}

fn tool_destination_rel(name: &str) -> Result<PathBuf, String> {
    let rel = sanitize_relative_path(name)?;
    let has_sep = rel.components().count() > 1;
    if has_sep {
        return Ok(rel);
    }
    let filename = rel
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| format!("invalid tool file name '{name}'"))?;
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".yaml") || lower.ends_with(".yml") || lower.ends_with(".toml") {
        let stem = Path::new(filename)
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or_default()
            .trim();
        if stem.is_empty() {
            return Err(format!("invalid tool manifest file name '{name}'"));
        }
        return Ok(PathBuf::from(stem).join("TOOL.yaml"));
    }
    Ok(rel)
}

fn write_bundle_file(root: &Path, rel: &Path, content: &str) -> Result<(), String> {
    let dst = root.join(rel);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    std::fs::write(&dst, content).map_err(|e| format!("failed to write {}: {e}", dst.display()))
}

fn install_bundle_files<F>(
    root: &Path,
    files: &[BundleFile],
    section: &str,
    errors: &mut Vec<String>,
    mut path_mapper: F,
) -> usize
where
    F: FnMut(&str) -> Result<PathBuf, String>,
{
    let mut count = 0usize;
    if let Err(e) = std::fs::create_dir_all(root) {
        errors.push(format!(
            "{section}: failed to create root directory {}: {e}",
            root.display()
        ));
        return 0;
    }
    for file in files {
        let rel = match path_mapper(&file.name) {
            Ok(rel) => rel,
            Err(e) => {
                errors.push(format!("{section}: rejected file '{}' ({e})", file.name));
                continue;
            }
        };
        match write_bundle_file(root, &rel, &file.content) {
            Ok(()) => count += 1,
            Err(e) => errors.push(format!("{section}: {e}")),
        }
    }
    count
}

/// POST /api/extensions/preview
pub async fn preview_handler(
    State(_state): State<AppState>,
    Json(req): Json<PreviewRequest>,
) -> impl IntoResponse {
    let source = match build_source(&req) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e})),
            )
                .into_response()
        }
    };
    match run_adapter(&req.source_kind, &source).await {
        Ok(bundle) => Json(bundle).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct InstallComponents {
    #[serde(default)]
    pub skills: bool,
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub workflows: bool,
    #[serde(default = "bool_true")]
    pub mcp: bool,
    #[serde(default = "bool_true")]
    pub rag: bool,
}
fn bool_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct InstallRequest {
    pub source_kind: String,
    pub source_ref: String,
    pub github_owner: Option<String>,
    pub github_repo: Option<String>,
    pub github_ref: Option<String>,
    pub components: Option<InstallComponents>,
}

#[derive(Deserialize)]
pub struct UninstallRequest {
    pub source_kind: String,
    pub source_ref: String,
    pub github_owner: Option<String>,
    pub github_repo: Option<String>,
    pub github_ref: Option<String>,
    pub components: Option<InstallComponents>,
}

/// POST /api/extensions/install
pub async fn install_handler(
    State(_state): State<AppState>,
    Json(req): Json<InstallRequest>,
) -> impl IntoResponse {
    let preview_req = PreviewRequest {
        source_kind: req.source_kind.clone(),
        source_ref: req.source_ref.clone(),
        github_owner: req.github_owner.clone(),
        github_repo: req.github_repo.clone(),
        github_ref: req.github_ref.clone(),
    };
    let source = match build_source(&preview_req) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e})),
            )
                .into_response()
        }
    };
    let bundle = match run_adapter(&req.source_kind, &source).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response()
        }
    };

    let comps = req.components.unwrap_or(InstallComponents {
        skills: true,
        tools: true,
        workflows: true,
        mcp: true,
        rag: true,
    });
    let home = agent007_write_home();
    let mut installed: HashMap<&str, usize> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();

    // Install skills
    if comps.skills {
        let dir = home.join("skills");
        let count = install_bundle_files(
            &dir,
            &bundle.skills,
            "skills",
            &mut errors,
            sanitize_relative_path,
        );
        installed.insert("skills", count);
    }
    // Install tools
    if comps.tools {
        let dir = home.join("tools");
        let count = install_bundle_files(
            &dir,
            &bundle.tools,
            "tools",
            &mut errors,
            tool_destination_rel,
        );
        installed.insert("tools", count);
    }
    // Install workflows
    if comps.workflows {
        let dir = home.join("workflows");
        let count = install_bundle_files(
            &dir,
            &bundle.workflows,
            "workflows",
            &mut errors,
            sanitize_relative_path,
        );
        installed.insert("workflows", count);
    }
    // Register MCP servers
    if comps.mcp {
        let mut count = 0;
        for srv in &bundle.mcp_servers {
            let add_req = AddMcpServerRequest {
                name: srv["name"].as_str().map(String::from),
                source_kind: srv["source_kind"].as_str().unwrap_or("manual").to_string(),
                source_ref: srv["source_ref"].as_str().unwrap_or("").to_string(),
                command: srv["command"].as_str().map(String::from),
                args: srv["args"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                env: Default::default(),
                scope: "project".to_string(),
            };
            match add_mcp_server(&home, add_req) {
                Ok(_) => count += 1,
                Err(e) => errors.push(format!(
                    "mcp_servers: failed to add '{}': {e}",
                    srv["name"].as_str().unwrap_or("<unnamed>")
                )),
            }
        }
        installed.insert("mcp_servers", count);
    }
    // Register RAG sources
    if comps.rag {
        let mut count = 0;
        for src in &bundle.rag_sources {
            let kind_str = src["kind"].as_str().unwrap_or("url");
            let kind = match kind_str {
                "file" => RagKind::File,
                "directory" => RagKind::Directory,
                _ => RagKind::Url,
            };
            let add_req = AddRagSourceRequest {
                name: src["name"].as_str().unwrap_or("source").to_string(),
                kind,
                source_ref: src["source_ref"].as_str().unwrap_or("").to_string(),
                scope: "project".to_string(),
                chunk_size: 512,
            };
            match add_rag_source(&home, add_req) {
                Ok(_) => count += 1,
                Err(e) => errors.push(format!(
                    "rag_sources: failed to add '{}': {e}",
                    src["name"].as_str().unwrap_or("<unnamed>")
                )),
            }
        }
        installed.insert("rag_sources", count);
    }

    // Record in installed.json
    let name = bundle
        .manifest
        .as_ref()
        .map(|m| m.extension.name.clone())
        .unwrap_or_else(|| req.source_ref.clone());
    if let Err(e) = record_installed(&home, &name, &req.source_kind, &req.source_ref, &bundle) {
        errors.push(format!("failed to record install metadata: {e}"));
    }

    if errors.is_empty() {
        Json(serde_json::json!({ "installed": installed, "extension": name })).into_response()
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "extension install completed with failures",
                "extension": name,
                "installed": installed,
                "errors": errors,
            })),
        )
            .into_response()
    }
}

fn record_installed(
    home: &std::path::Path,
    name: &str,
    source_kind: &str,
    source_ref: &str,
    bundle: &ExtensionBundle,
) -> Result<(), String> {
    let path = home.join("extensions").join("installed.json");
    std::fs::create_dir_all(
        path.parent()
            .ok_or_else(|| "invalid installed.json path parent".to_string())?,
    )
    .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    let mut list: Vec<serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .map(|s| serde_json::from_str(&s).map_err(|e| format!("invalid installed.json: {e}")))
        .transpose()?
        .unwrap_or_default();
    list.retain(|e| e["name"].as_str() != Some(name));
    list.push(serde_json::json!({
        "name": name,
        "source_kind": source_kind,
        "source_ref": source_ref,
        "version": bundle.manifest.as_ref().map(|m| m.extension.version.clone()).unwrap_or_default(),
        "compat_grade": bundle.compat_grade.as_ref().map(|g| g.to_string()).unwrap_or_default(),
        "skills": bundle.skills.len(),
        "tools": bundle.tools.len(),
        "workflows": bundle.workflows.len(),
        "mcp_servers": bundle.mcp_servers.len(),
        "rag_sources": bundle.rag_sources.len(),
        "installed_at": chrono::Utc::now().to_rfc3339(),
    }));
    let serialized = serde_json::to_string_pretty(&list)
        .map_err(|e| format!("failed to serialize installed list: {e}"))?;
    std::fs::write(&path, serialized)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(())
}

fn remove_bundle_files<F>(root: &Path, files: &[BundleFile], mut path_mapper: F) -> usize
where
    F: FnMut(&str) -> Result<PathBuf, String>,
{
    let mut removed = 0usize;
    for file in files {
        let Ok(rel) = path_mapper(&file.name) else {
            continue;
        };
        let dst = root.join(rel);
        if !dst.exists() {
            continue;
        }
        if dst.is_dir() {
            if std::fs::remove_dir_all(&dst).is_ok() {
                removed += 1;
            }
        } else if std::fs::remove_file(&dst).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn remove_installed_record(home: &Path, source_kind: &str, source_ref: &str) -> Result<(), String> {
    let path = home.join("extensions").join("installed.json");
    let mut list: Vec<serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let before = list.len();
    list.retain(|entry| {
        let kind = entry["source_kind"].as_str().unwrap_or_default();
        let src = entry["source_ref"].as_str().unwrap_or_default();
        !(kind == source_kind && src == source_ref)
    });
    if list.len() != before {
        let serialized = serde_json::to_string_pretty(&list)
            .map_err(|e| format!("failed to serialize installed list: {e}"))?;
        std::fs::write(&path, serialized)
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// POST /api/extensions/uninstall
pub async fn uninstall_handler(
    State(_state): State<AppState>,
    Json(req): Json<UninstallRequest>,
) -> impl IntoResponse {
    let preview_req = PreviewRequest {
        source_kind: req.source_kind.clone(),
        source_ref: req.source_ref.clone(),
        github_owner: req.github_owner.clone(),
        github_repo: req.github_repo.clone(),
        github_ref: req.github_ref.clone(),
    };
    let source = match build_source(&preview_req) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e})),
            )
                .into_response()
        }
    };
    let bundle = match run_adapter(&req.source_kind, &source).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response()
        }
    };

    let comps = req.components.unwrap_or(InstallComponents {
        skills: true,
        tools: true,
        workflows: true,
        mcp: true,
        rag: true,
    });
    let home = agent007_write_home();
    let mut removed: HashMap<&str, usize> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();

    if comps.skills {
        let count =
            remove_bundle_files(&home.join("skills"), &bundle.skills, sanitize_relative_path);
        removed.insert("skills", count);
    }
    if comps.tools {
        let count = remove_bundle_files(&home.join("tools"), &bundle.tools, tool_destination_rel);
        removed.insert("tools", count);
    }
    if comps.workflows {
        let count = remove_bundle_files(
            &home.join("workflows"),
            &bundle.workflows,
            sanitize_relative_path,
        );
        removed.insert("workflows", count);
    }
    if comps.mcp {
        let mut count = 0usize;
        for srv in &bundle.mcp_servers {
            if let Some(name) = srv["name"].as_str() {
                match delete_mcp_server(&home, name) {
                    Ok(()) => count += 1,
                    Err(e) => errors.push(format!("mcp_servers: failed to remove '{name}': {e}")),
                }
            }
        }
        removed.insert("mcp_servers", count);
    }
    if comps.rag {
        let mut count = 0usize;
        let existing = load_rag_sources(&home).unwrap_or_default();
        for src in &bundle.rag_sources {
            if let Some(name) = src["name"].as_str() {
                if let Some(found) = existing.iter().find(|s| s.name == name) {
                    match delete_rag_source(&home, &found.id) {
                        Ok(()) => count += 1,
                        Err(e) => {
                            errors.push(format!("rag_sources: failed to remove '{name}': {e}"))
                        }
                    }
                }
            }
        }
        removed.insert("rag_sources", count);
    }

    if let Err(e) = remove_installed_record(&home, &req.source_kind, &req.source_ref) {
        errors.push(format!("failed to update installed metadata: {e}"));
    }

    if errors.is_empty() {
        Json(serde_json::json!({ "removed": removed })).into_response()
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "extension uninstall completed with failures",
                "removed": removed,
                "errors": errors,
            })),
        )
            .into_response()
    }
}

/// GET /api/extensions/list
pub async fn list_handler(State(_state): State<AppState>) -> impl IntoResponse {
    let home = agent007_write_home();
    let path = home.join("extensions").join("installed.json");
    let list: Vec<serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    Json(serde_json::json!({ "extensions": list })).into_response()
}

#[cfg(test)]
mod tests {
    use super::{install_bundle_files, sanitize_relative_path, tool_destination_rel};
    use agent007_extensions::bundle::BundleFile;

    #[test]
    fn sanitize_rejects_parent_traversal() {
        let err = sanitize_relative_path("../escape.txt").unwrap_err();
        assert!(err.contains("parent traversal"));
    }

    #[test]
    fn tool_destination_maps_simple_manifest_name() {
        let mapped = tool_destination_rel("adb-flash.yaml").unwrap();
        assert_eq!(mapped.to_string_lossy(), "adb-flash/TOOL.yaml");
    }

    #[test]
    fn install_bundle_files_blocks_escape_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("skills");
        let outside = tmp.path().join("escape.md");
        let files = vec![
            BundleFile {
                name: "../escape.md".to_string(),
                content: "nope".to_string(),
            },
            BundleFile {
                name: "ok.md".to_string(),
                content: "ok".to_string(),
            },
        ];
        let mut errors = Vec::new();
        let written =
            install_bundle_files(&root, &files, "skills", &mut errors, sanitize_relative_path);
        assert_eq!(written, 1);
        assert!(root.join("ok.md").is_file());
        assert!(!outside.exists());
        assert!(!errors.is_empty());
    }
}
