use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const TOOL_STATE_FILE: &str = "TOOL.state.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct ToolState {
    source_kind: String,
    source_ref: String,
    source_version: Option<String>,
    imported_at: Option<String>,
    approval_required: bool,
    approved: bool,
    approved_by: Option<String>,
    approved_at: Option<String>,
    hash_pinning: bool,
    pinned_sha256: Option<String>,
    auto_skill_trigger: Option<String>,
    notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ToolImportRequest {
    pub provider: String,
    pub package: String,
    pub scope: Option<String>,
    pub version: Option<String>,
    pub executable: Option<String>,
    pub local_path: Option<String>,
    pub repo_url: Option<String>,
    pub entrypoint: Option<String>,
    pub safety: Option<String>,
    pub timeout_sec: Option<u64>,
    pub approval_required: Option<bool>,
    pub hash_pinning: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSearchResult {
    pub provider: String,
    pub package: String,
    pub version: Option<String>,
    pub description: String,
    pub homepage: Option<String>,
    pub score: Option<f64>,
    pub downloads: Option<u64>,
    /// Absolute path if the executable is already on $PATH.
    pub installed_path: Option<String>,
    /// Human-readable install command suggestion.
    pub install_cmd: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredTool {
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub category: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ToolScope {
    Project,
    Global,
}

impl ToolScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolScope::Project => "project",
            ToolScope::Global => "global",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "project" | "proj" | "p" => Some(Self::Project),
            "global" | "g" => Some(Self::Global),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ToolRuntime {
    Shell,
    Python,
    Node,
    Binary,
}

impl ToolRuntime {
    fn infer_from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "sh" | "bash" | "zsh" | "ps1" | "bat" | "cmd" => Self::Shell,
            "py" => Self::Python,
            "js" | "mjs" | "cjs" | "ts" => Self::Node,
            _ => Self::Binary,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ToolArgSpec {
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub arg_type: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolManifest {
    pub name: String,
    pub description: String,
    pub runtime: ToolRuntime,
    pub entrypoint: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub timeout_sec: u64,
    #[serde(default)]
    pub safety: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub args: Vec<ToolArgSpec>,
    #[serde(default)]
    pub working_dir: Option<String>,
}

impl Default for ToolManifest {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            runtime: ToolRuntime::Shell,
            entrypoint: String::new(),
            command: None,
            timeout_sec: 60,
            safety: "readonly".to_string(),
            tags: Vec::new(),
            args: Vec::new(),
            working_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolRecord {
    pub name: String,
    pub description: String,
    pub runtime: ToolRuntime,
    pub scope: String,
    pub source: String,
    pub format: String,
    pub timeout_sec: u64,
    pub safety: String,
    pub tags: Vec<String>,
    pub entrypoint: String,
    pub entry_path: String,
    pub manifest_path: Option<String>,
    pub precedence_source: String,
    pub has_collisions: bool,
    pub collision_count: u64,
    pub shadowed_sources: Vec<String>,
    pub args: Vec<ToolArgSpec>,
    pub updated_at: Option<String>,
    pub source_kind: String,
    pub source_ref: String,
    pub source_version: Option<String>,
    pub approval_required: bool,
    pub approved: bool,
    pub approved_by: Option<String>,
    pub approved_at: Option<String>,
    pub hash_pinning: bool,
    pub pinned_sha256: Option<String>,
    pub current_sha256: Option<String>,
    pub hash_match: Option<bool>,
    pub auto_skill_trigger: Option<String>,
}

#[derive(Debug, Clone)]
struct ToolDescriptor {
    name: String,
    manifest: ToolManifest,
    scope: ToolScope,
    source: String,
    format: String,
    entry_path: PathBuf,
    manifest_path: Option<PathBuf>,
    tool_root: PathBuf,
    updated_at: Option<String>,
    state: ToolState,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTestResult {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
    pub command: Vec<String>,
}

fn manifest_file_names() -> &'static [&'static str] {
    &[
        "TOOL.yaml",
        "tool.yaml",
        "TOOL.yml",
        "tool.yml",
        "TOOL.toml",
        "tool.toml",
    ]
}

fn is_script_like(path: &Path) -> bool {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "sh" | "bash" | "zsh" | "py" | "js" | "mjs" | "cjs" | "ts" | "ps1" | "bat" | "cmd" => true,
        _ => false,
    }
}

fn find_manifest_path(dir: &Path) -> Option<PathBuf> {
    manifest_file_names()
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

fn state_file_path(tool_root: &Path) -> PathBuf {
    tool_root.join(TOOL_STATE_FILE)
}

fn load_state(tool_root: &Path) -> ToolState {
    let path = state_file_path(tool_root);
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return ToolState::default(),
    };
    serde_json::from_str::<ToolState>(&raw).unwrap_or_default()
}

fn save_state(tool_root: &Path, state: &ToolState) -> Result<(), String> {
    let path = state_file_path(tool_root);
    let raw = serde_json::to_string_pretty(state)
        .map_err(|e| format!("failed to serialize tool state: {e}"))?;
    std::fs::write(&path, raw).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    let hash = hasher.finalize();
    Ok(hash.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn default_state_for_manual() -> ToolState {
    ToolState {
        source_kind: "manual".to_string(),
        source_ref: String::new(),
        source_version: None,
        imported_at: None,
        approval_required: false,
        approved: true,
        approved_by: Some("system".to_string()),
        approved_at: Some(Utc::now().to_rfc3339()),
        hash_pinning: false,
        pinned_sha256: None,
        auto_skill_trigger: None,
        notes: Vec::new(),
    }
}

fn parse_manifest(path: &Path) -> Result<ToolManifest, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut manifest: ToolManifest = if ext == "toml" {
        toml::from_str(&content).map_err(|e| format!("invalid manifest {}: {e}", path.display()))?
    } else {
        serde_yaml::from_str(&content)
            .map_err(|e| format!("invalid manifest {}: {e}", path.display()))?
    };
    normalize_manifest(&mut manifest)?;
    Ok(manifest)
}

fn normalize_manifest(manifest: &mut ToolManifest) -> Result<(), String> {
    manifest.name = manifest.name.trim().to_string();
    manifest.description = manifest.description.trim().to_string();
    manifest.entrypoint = manifest.entrypoint.trim().to_string();
    if manifest.name.is_empty() {
        return Err("manifest name cannot be empty".to_string());
    }
    if manifest.entrypoint.is_empty() && manifest.command.is_none() {
        return Err(format!(
            "manifest '{}' must define entrypoint or command",
            manifest.name
        ));
    }
    if manifest.timeout_sec == 0 {
        manifest.timeout_sec = 60;
    }
    if manifest.timeout_sec > 3600 {
        return Err(format!(
            "manifest '{}' timeout_sec must be <= 3600",
            manifest.name
        ));
    }
    if manifest.safety.trim().is_empty() {
        manifest.safety = "readonly".to_string();
    }

    for arg in &mut manifest.args {
        arg.name = arg.name.trim().to_string();
        arg.description = arg.description.trim().to_string();
        arg.arg_type = arg.arg_type.trim().to_ascii_lowercase();
        if arg.arg_type.is_empty() {
            arg.arg_type = "string".to_string();
        }
        if arg.name.is_empty() {
            return Err(format!(
                "manifest '{}' has an arg with empty name",
                manifest.name
            ));
        }
        match arg.arg_type.as_str() {
            "string" | "number" | "boolean" => {}
            other => {
                return Err(format!(
                    "manifest '{}' arg '{}' has unsupported type '{other}'",
                    manifest.name, arg.name
                ));
            }
        }
    }

    Ok(())
}

fn tool_scope_dirs(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
) -> Vec<(ToolScope, PathBuf)> {
    let mut dirs = Vec::new();
    dirs.push((ToolScope::Global, global_home.join("tools")));
    if let Some(project_home) = project_home {
        dirs.push((ToolScope::Project, project_home.join("tools")));
    }
    dirs
}

fn collect_scope_tools(scope: ToolScope, tools_dir: &Path) -> Result<Vec<ToolDescriptor>, String> {
    if !tools_dir.exists() {
        return Ok(Vec::new());
    }
    let mut tools = Vec::new();
    let entries = std::fs::read_dir(tools_dir)
        .map_err(|e| format!("failed to read {}: {e}", tools_dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(manifest_path) = find_manifest_path(&path) {
                let mut manifest = parse_manifest(&manifest_path)?;
                if manifest.name.trim().is_empty() {
                    if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                        manifest.name = name.to_string();
                    }
                }

                let entry_path = if let Some(command) = &manifest.command {
                    PathBuf::from(command)
                } else {
                    path.join(&manifest.entrypoint)
                };

                tools.push(ToolDescriptor {
                    name: manifest.name.to_ascii_lowercase(),
                    manifest,
                    scope,
                    source: scope.as_str().to_string(),
                    format: "manifest".to_string(),
                    entry_path,
                    manifest_path: Some(manifest_path.clone()),
                    tool_root: path.clone(),
                    updated_at: std::fs::metadata(&manifest_path)
                        .ok()
                        .and_then(|meta| meta.modified().ok())
                        .map(|time| chrono::DateTime::<Utc>::from(time).to_rfc3339()),
                    state: load_state(&path),
                });
            }
            continue;
        }

        if !path.is_file() || !is_script_like(&path) {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(filename) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };

        let manifest = ToolManifest {
            name: stem.to_string(),
            description: format!("Legacy tool script '{}'", filename),
            runtime: ToolRuntime::infer_from_path(&path),
            entrypoint: filename.to_string(),
            command: None,
            timeout_sec: 60,
            safety: "readonly".to_string(),
            tags: vec!["legacy".to_string()],
            args: Vec::new(),
            working_dir: None,
        };

        tools.push(ToolDescriptor {
            name: stem.to_ascii_lowercase(),
            manifest,
            scope,
            source: scope.as_str().to_string(),
            format: "legacy".to_string(),
            entry_path: path.clone(),
            manifest_path: None,
            tool_root: tools_dir.to_path_buf(),
            updated_at: std::fs::metadata(&path)
                .ok()
                .and_then(|meta| meta.modified().ok())
                .map(|time| chrono::DateTime::<Utc>::from(time).to_rfc3339()),
            state: default_state_for_manual(),
        });
    }

    Ok(tools)
}

fn to_api_record(
    descriptor: &ToolDescriptor,
    precedence_source: ToolScope,
    shadowed_sources: Vec<ToolScope>,
) -> ToolRecord {
    let current_sha256 = if descriptor.entry_path.is_file() {
        sha256_file(&descriptor.entry_path).ok()
    } else {
        None
    };
    let hash_match = if descriptor.state.hash_pinning {
        match (
            descriptor.state.pinned_sha256.as_ref(),
            current_sha256.as_ref(),
        ) {
            (Some(expected), Some(actual)) => Some(expected.eq_ignore_ascii_case(actual)),
            _ => Some(false),
        }
    } else {
        None
    };
    ToolRecord {
        name: descriptor.manifest.name.clone(),
        description: descriptor.manifest.description.clone(),
        runtime: descriptor.manifest.runtime,
        scope: descriptor.source.clone(),
        source: descriptor.source.clone(),
        format: descriptor.format.clone(),
        timeout_sec: descriptor.manifest.timeout_sec,
        safety: descriptor.manifest.safety.clone(),
        tags: descriptor.manifest.tags.clone(),
        entrypoint: descriptor.manifest.entrypoint.clone(),
        entry_path: descriptor.entry_path.display().to_string(),
        manifest_path: descriptor
            .manifest_path
            .as_ref()
            .map(|path| path.display().to_string()),
        precedence_source: precedence_source.as_str().to_string(),
        has_collisions: !shadowed_sources.is_empty(),
        collision_count: shadowed_sources.len() as u64,
        shadowed_sources: shadowed_sources
            .into_iter()
            .map(|scope| scope.as_str().to_string())
            .collect(),
        args: descriptor.manifest.args.clone(),
        updated_at: descriptor.updated_at.clone(),
        source_kind: if descriptor.state.source_kind.trim().is_empty() {
            "manual".to_string()
        } else {
            descriptor.state.source_kind.clone()
        },
        source_ref: descriptor.state.source_ref.clone(),
        source_version: descriptor.state.source_version.clone(),
        approval_required: descriptor.state.approval_required,
        approved: descriptor.state.approved,
        approved_by: descriptor.state.approved_by.clone(),
        approved_at: descriptor.state.approved_at.clone(),
        hash_pinning: descriptor.state.hash_pinning,
        pinned_sha256: descriptor.state.pinned_sha256.clone(),
        current_sha256,
        hash_match,
        auto_skill_trigger: descriptor.state.auto_skill_trigger.clone(),
    }
}

pub fn list_tools(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
) -> Result<Vec<ToolRecord>, String> {
    let mut all: Vec<ToolDescriptor> = Vec::new();
    for (scope, dir) in tool_scope_dirs(project_home, global_home) {
        all.extend(collect_scope_tools(scope, &dir)?);
    }

    let mut grouped: BTreeMap<String, Vec<ToolDescriptor>> = BTreeMap::new();
    for item in all {
        grouped.entry(item.name.clone()).or_default().push(item);
    }

    let mut records = Vec::new();
    for (_key, mut candidates) in grouped {
        candidates.sort_by_key(|entry| match entry.scope {
            ToolScope::Global => 0,
            ToolScope::Project => 1,
        });

        let winner = candidates
            .iter()
            .max_by_key(|entry| match entry.scope {
                ToolScope::Global => 0,
                ToolScope::Project => 1,
            })
            .expect("at least one candidate");

        let shadowed: Vec<ToolScope> = candidates
            .iter()
            .filter(|entry| entry.scope != winner.scope)
            .map(|entry| entry.scope)
            .collect();

        records.push(to_api_record(winner, winner.scope, shadowed));
    }

    records.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    Ok(records)
}

pub fn get_tool_by_name(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
    name: &str,
) -> Result<Option<ToolRecord>, String> {
    let key = name.trim().to_ascii_lowercase();
    if key.is_empty() {
        return Ok(None);
    }
    let tools = list_tools(project_home, global_home)?;
    Ok(tools
        .into_iter()
        .find(|tool| tool.name.to_ascii_lowercase() == key))
}

fn sanitize_rel_path(value: &str) -> Result<PathBuf, String> {
    let raw = value.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err("entrypoint cannot be empty".to_string());
    }
    let rel = PathBuf::from(raw.clone());
    if rel.is_absolute() {
        return Err("absolute entrypoint paths are not allowed".to_string());
    }
    for component in rel.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("entrypoint cannot contain '..'".to_string());
        }
    }
    Ok(rel)
}

pub fn save_manifest_tool(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
    scope: ToolScope,
    mut manifest: ToolManifest,
    entry_content: Option<String>,
    overwrite: bool,
) -> Result<ToolRecord, String> {
    normalize_manifest(&mut manifest)?;
    let global_home_for_lookup = global_home.clone();

    let home = match scope {
        ToolScope::Project => project_home.clone().ok_or_else(|| {
            "project scope is unavailable: not inside a project with .agent007".to_string()
        })?,
        ToolScope::Global => global_home,
    };
    let tools_dir = home.join("tools");
    std::fs::create_dir_all(&tools_dir)
        .map_err(|e| format!("failed to create {}: {e}", tools_dir.display()))?;

    let tool_slug = manifest
        .name
        .trim()
        .trim_start_matches('/')
        .to_ascii_lowercase()
        .replace(' ', "-");
    if tool_slug.is_empty() {
        return Err("manifest name cannot be empty".to_string());
    }

    let tool_dir = tools_dir.join(&tool_slug);
    if tool_dir.exists() && !tool_dir.is_dir() {
        return Err(format!(
            "tool path {} exists but is not a directory",
            tool_dir.display()
        ));
    }
    if !tool_dir.exists() {
        std::fs::create_dir_all(&tool_dir)
            .map_err(|e| format!("failed to create {}: {e}", tool_dir.display()))?;
    }

    let manifest_path = tool_dir.join("TOOL.yaml");
    if manifest_path.exists() && !overwrite {
        return Err(format!(
            "manifest {} already exists (set overwrite=true to replace)",
            manifest_path.display()
        ));
    }

    let entry_rel = sanitize_rel_path(&manifest.entrypoint)?;
    let entry_path = tool_dir.join(&entry_rel);
    if let Some(content) = entry_content {
        if let Some(parent) = entry_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
        if entry_path.exists() && !overwrite {
            return Err(format!(
                "entrypoint {} already exists (set overwrite=true to replace)",
                entry_path.display()
            ));
        }
        std::fs::write(&entry_path, content)
            .map_err(|e| format!("failed to write {}: {e}", entry_path.display()))?;
    }

    let yaml = serde_yaml::to_string(&manifest)
        .map_err(|e| format!("failed to serialize manifest: {e}"))?;
    std::fs::write(&manifest_path, yaml)
        .map_err(|e| format!("failed to write {}: {e}", manifest_path.display()))?;

    let mut state = load_state(&tool_dir);
    if state.source_kind.trim().is_empty() {
        state = default_state_for_manual();
    }
    if !state.approval_required && !state.approved {
        state.approved = true;
        state.approved_at = Some(Utc::now().to_rfc3339());
        state.approved_by = Some("system".to_string());
    }
    if state.hash_pinning && state.pinned_sha256.is_none() && entry_path.is_file() {
        state.pinned_sha256 = sha256_file(&entry_path).ok();
    }
    save_state(&tool_dir, &state)?;

    get_tool_by_name(project_home, global_home_for_lookup, &manifest.name)
        .and_then(|opt| opt.ok_or_else(|| "saved tool not found after write".to_string()))
}

pub fn delete_tool(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
    name: &str,
    scope: ToolScope,
) -> Result<bool, String> {
    let name_key = name.trim().to_ascii_lowercase();
    if name_key.is_empty() {
        return Err("tool name cannot be empty".to_string());
    }

    let home = match scope {
        ToolScope::Project => project_home.ok_or_else(|| {
            "project scope is unavailable: not inside a project with .agent007".to_string()
        })?,
        ToolScope::Global => global_home,
    };

    let tools_dir = home.join("tools");
    if !tools_dir.exists() {
        return Ok(false);
    }

    let mut removed = false;
    for entry in std::fs::read_dir(&tools_dir)
        .map_err(|e| format!("failed to read {}: {e}", tools_dir.display()))?
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            let manifest_path = find_manifest_path(&path);
            if let Some(manifest_path) = manifest_path {
                if let Ok(manifest) = parse_manifest(&manifest_path) {
                    if manifest.name.to_ascii_lowercase() == name_key {
                        std::fs::remove_dir_all(&path)
                            .map_err(|e| format!("failed to remove {}: {e}", path.display()))?;
                        removed = true;
                        break;
                    }
                }
            }
            if let Some(dir_name) = path.file_name().and_then(|value| value.to_str()) {
                if dir_name.to_ascii_lowercase() == name_key {
                    std::fs::remove_dir_all(&path)
                        .map_err(|e| format!("failed to remove {}: {e}", path.display()))?;
                    removed = true;
                    break;
                }
            }
            continue;
        }

        if path.is_file()
            && is_script_like(&path)
            && path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(|stem| stem.to_ascii_lowercase() == name_key)
                .unwrap_or(false)
        {
            std::fs::remove_file(&path)
                .map_err(|e| format!("failed to remove {}: {e}", path.display()))?;
            removed = true;
            break;
        }
    }

    Ok(removed)
}

fn render_manifest_args(
    manifest: &ToolManifest,
    args: Option<Value>,
) -> Result<Vec<String>, String> {
    if manifest.args.is_empty() {
        return match args {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .into_iter()
                .map(|item| match item {
                    Value::String(text) => Ok(text),
                    other => Err(format!("array args must be strings, got {other}")),
                })
                .collect(),
            Some(Value::Object(obj)) => {
                let mut rendered = Vec::new();
                let mut keys: Vec<String> = obj.keys().cloned().collect();
                keys.sort();
                for key in keys {
                    let value = obj.get(&key).cloned().unwrap_or(Value::Null);
                    let arg_key = format!("--{}", key.replace('_', "-"));
                    rendered.push(arg_key);
                    rendered.push(value_to_cli(&value)?);
                }
                Ok(rendered)
            }
            Some(other) => Err(format!(
                "args must be array|string map when manifest args is empty, got {other}"
            )),
        };
    }

    let provided = match args {
        Some(Value::Object(map)) => map,
        Some(Value::Null) | None => serde_json::Map::new(),
        Some(other) => {
            return Err(format!(
                "args must be an object when manifest defines args schema, got {other}"
            ))
        }
    };

    let mut rendered = Vec::new();
    for spec in &manifest.args {
        let value = provided.get(&spec.name).cloned();
        if value.is_none() {
            if spec.required {
                return Err(format!("missing required arg '{}'", spec.name));
            }
            continue;
        }
        let value = value.unwrap();
        validate_arg_type(&spec.arg_type, &value, &spec.name)?;
        rendered.push(value_to_cli(&value)?);
    }

    Ok(rendered)
}

fn validate_arg_type(kind: &str, value: &Value, name: &str) -> Result<(), String> {
    match kind {
        "string" => {
            if !value.is_string() {
                return Err(format!("arg '{name}' must be string"));
            }
        }
        "number" => {
            if !value.is_number() {
                return Err(format!("arg '{name}' must be number"));
            }
        }
        "boolean" => {
            if !value.is_boolean() {
                return Err(format!("arg '{name}' must be boolean"));
            }
        }
        _ => return Err(format!("arg '{name}' has unsupported type '{kind}'")),
    }
    Ok(())
}

fn value_to_cli(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(if *flag { "true" } else { "false" }.to_string()),
        Value::Null => Ok("".to_string()),
        _ => Err(format!("unsupported arg value type: {value}")),
    }
}

fn resolve_tool_descriptor(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
    name: &str,
) -> Result<Option<ToolDescriptor>, String> {
    let target = name.trim().to_ascii_lowercase();
    if target.is_empty() {
        return Ok(None);
    }

    let mut all: Vec<ToolDescriptor> = Vec::new();
    for (scope, dir) in tool_scope_dirs(project_home, global_home) {
        all.extend(collect_scope_tools(scope, &dir)?);
    }

    let mut matches: Vec<ToolDescriptor> = all
        .into_iter()
        .filter(|entry| entry.name == target || entry.manifest.name.to_ascii_lowercase() == target)
        .collect();

    matches.sort_by_key(|entry| match entry.scope {
        ToolScope::Global => 0,
        ToolScope::Project => 1,
    });

    Ok(matches.pop())
}

fn resolve_scope_home(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
    scope: ToolScope,
) -> Result<PathBuf, String> {
    match scope {
        ToolScope::Project => project_home
            .ok_or_else(|| {
                "project scope is unavailable: not inside a project with .agent007".to_string()
            })
            .map(|p| p.join("tools")),
        ToolScope::Global => Ok(global_home.join("tools")),
    }
}

fn slugify_tool_name(raw: &str, fallback: &str) -> String {
    let slug = raw
        .trim()
        .to_ascii_lowercase()
        .replace('\\', "/")
        .split('/')
        .next_back()
        .unwrap_or(raw)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}

fn provider_script_crates(pkg: &str, version: Option<&str>, executable: &str) -> String {
    let spec = if let Some(version) = version.filter(|v| !v.trim().is_empty()) {
        format!("{pkg}@{}", version.trim())
    } else {
        pkg.to_string()
    };
    format!(
        "#!/usr/bin/env sh\nset -e\nif ! command -v {executable} >/dev/null 2>&1; then\n  cargo install --locked {spec}\nfi\nexec {executable} \"$@\"\n"
    )
}

/// Returns the absolute path of `binary` if it exists anywhere on $PATH.
fn which_binary(binary: &str) -> Option<String> {
    let output = std::process::Command::new("which")
        .arg(binary)
        .output()
        .ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }
    None
}

/// Returns the first line of `binary --version` / `binary -V`, if available.
fn probe_version(binary: &str) -> Option<String> {
    for flag in &["--version", "-V", "version"] {
        if let Ok(out) = std::process::Command::new(binary).arg(flag).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Some(line) = s.lines().next() {
                    let v = line.trim().to_string();
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
        }
    }
    None
}

fn provider_script_npm(pkg: &str, version: Option<&str>, executable: &str) -> String {
    let spec = if let Some(version) = version.filter(|v| !v.trim().is_empty()) {
        format!("{pkg}@{}", version.trim())
    } else {
        pkg.to_string()
    };
    format!(
        "#!/usr/bin/env sh\nset -e\nif ! command -v {executable} >/dev/null 2>&1; then\n  npm install -g {spec}\nfi\nexec {executable} \"$@\"\n"
    )
}

fn provider_script_github(repo: &str, entrypoint: &str) -> String {
    format!(
        "#!/usr/bin/env sh\nset -e\nROOT=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\nVENDOR=\"$ROOT/vendor\"\nREPO_DIR=\"$VENDOR/repo\"\nif [ ! -d \"$REPO_DIR/.git\" ]; then\n  mkdir -p \"$VENDOR\"\n  git clone --depth 1 {repo} \"$REPO_DIR\"\nfi\nexec \"$REPO_DIR/{entrypoint}\" \"$@\"\n"
    )
}

fn copy_with_exec_bits(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    std::fs::copy(src, dst)
        .map_err(|e| format!("failed to copy {} to {}: {e}", src.display(), dst.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(dst)
            .map_err(|e| format!("failed to stat {}: {e}", dst.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(dst, perms)
            .map_err(|e| format!("failed to chmod {}: {e}", dst.display()))?;
    }
    Ok(())
}

fn provider_runtime_from_name(name: &str) -> ToolRuntime {
    ToolRuntime::infer_from_path(Path::new(name))
}

pub async fn search_remote_tools(
    provider: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<ToolSearchResult>, String> {
    let provider = provider.trim().to_ascii_lowercase();
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 50);
    let client = reqwest::Client::builder()
        .user_agent("agent007-tool-registry/0.1")
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to create http client: {e}"))?;

    let mut out = Vec::new();
    let include = |name: &str| provider == "all" || provider == name;

    // Infer the executable name from the query (last path segment, no slashes).
    let exe_guess = query.split('/').next_back().unwrap_or(query);

    if include("crates") {
        let url = "https://crates.io/api/v1/crates";
        if let Ok(resp) = client
            .get(url)
            .query(&[("q", query), ("per_page", &limit.to_string())])
            .send()
            .await
        {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(items) = body.get("crates").and_then(|v| v.as_array()) {
                    for item in items {
                        let package = item
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        if package.is_empty() {
                            continue;
                        }
                        let installed_path =
                            which_binary(&package).or_else(|| which_binary(exe_guess));
                        let install_cmd = Some(format!("cargo install {package}"));
                        out.push(ToolSearchResult {
                            provider: "crates".to_string(),
                            package: package.clone(),
                            version: item
                                .get("max_version")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            description: item
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            homepage: item
                                .get("homepage")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            score: None,
                            downloads: item.get("downloads").and_then(|v| v.as_u64()),
                            installed_path,
                            install_cmd,
                        });
                    }
                }
            }
        }
    }

    if include("npm") {
        let url = "https://registry.npmjs.org/-/v1/search";
        if let Ok(resp) = client
            .get(url)
            .query(&[("text", query), ("size", &limit.to_string())])
            .send()
            .await
        {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(items) = body.get("objects").and_then(|v| v.as_array()) {
                    for item in items {
                        let pkg = item.get("package").cloned().unwrap_or(Value::Null);
                        let package = pkg
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        if package.is_empty() {
                            continue;
                        }
                        let bin_name = package
                            .trim_start_matches('@')
                            .split('/')
                            .next_back()
                            .unwrap_or(&package)
                            .to_string();
                        let installed_path =
                            which_binary(&bin_name).or_else(|| which_binary(exe_guess));
                        let install_cmd = Some(format!("npm install -g {package}"));
                        out.push(ToolSearchResult {
                            provider: "npm".to_string(),
                            package: package.clone(),
                            version: pkg
                                .get("version")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            description: pkg
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            homepage: pkg
                                .get("links")
                                .and_then(|l| l.get("homepage"))
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            score: item
                                .get("score")
                                .and_then(|s| s.get("final"))
                                .and_then(|v| v.as_f64()),
                            downloads: None,
                            installed_path,
                            install_cmd,
                        });
                    }
                }
            }
        }
    }

    if include("github") {
        let url = "https://api.github.com/search/repositories";
        if let Ok(resp) = client
            .get(url)
            .query(&[("q", query), ("per_page", &limit.to_string())])
            .send()
            .await
        {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(items) = body.get("items").and_then(|v| v.as_array()) {
                    for item in items {
                        let package = item
                            .get("full_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        if package.is_empty() {
                            continue;
                        }
                        let repo_name = package
                            .split('/')
                            .next_back()
                            .unwrap_or(&package)
                            .to_string();
                        let installed_path =
                            which_binary(&repo_name).or_else(|| which_binary(exe_guess));
                        out.push(ToolSearchResult {
                            provider: "github".to_string(),
                            package: package.clone(),
                            version: None,
                            description: item
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            homepage: item
                                .get("html_url")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            score: item.get("score").and_then(|v| v.as_f64()),
                            downloads: item.get("stargazers_count").and_then(|v| v.as_u64()),
                            installed_path,
                            install_cmd: None,
                        });
                    }
                }
            }
        }
    }

    // Homebrew: try exact formula lookup then fuzzy via search endpoint.
    if include("brew") {
        // Exact lookup first.
        let formula_url = format!("https://formulae.brew.sh/api/formula/{}.json", query);
        if let Ok(resp) = client.get(&formula_url).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    let name = body
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(query)
                        .to_string();
                    let desc = body
                        .get("desc")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let version = body
                        .get("versions")
                        .and_then(|v| v.get("stable"))
                        .and_then(|v| v.as_str())
                        .map(|v| v.to_string());
                    let installed_path = which_binary(&name).or_else(|| which_binary(exe_guess));
                    out.push(ToolSearchResult {
                        provider: "brew".to_string(),
                        package: name.clone(),
                        version,
                        description: desc,
                        homepage: body
                            .get("homepage")
                            .and_then(|v| v.as_str())
                            .map(|v| v.to_string()),
                        score: None,
                        downloads: body
                            .get("analytics")
                            .and_then(|a| a.get("install"))
                            .and_then(|i| i.get("30d"))
                            .and_then(|d| d.get(&name))
                            .and_then(|v| v.as_u64()),
                        installed_path,
                        install_cmd: Some(format!("brew install {name}")),
                    });
                }
            }
        }
        // Broader search via Homebrew search endpoint (returns formula names only).
        let search_url = format!("https://formulae.brew.sh/api/formula.json");
        if out.iter().filter(|r| r.provider == "brew").count() < 3 {
            if let Ok(resp) = client.get(&search_url).send().await {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(formulae) = body.as_array() {
                        let q_lower = query.to_ascii_lowercase();
                        let mut added = 0usize;
                        for formula in formulae {
                            if added >= 5 {
                                break;
                            }
                            let name = formula
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            let desc = formula
                                .get("desc")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            if name.to_ascii_lowercase().contains(&q_lower)
                                || desc.to_ascii_lowercase().contains(&q_lower)
                            {
                                if out
                                    .iter()
                                    .any(|r| r.provider == "brew" && r.package == name)
                                {
                                    continue;
                                }
                                let installed_path =
                                    which_binary(name).or_else(|| which_binary(exe_guess));
                                let version = formula
                                    .get("versions")
                                    .and_then(|v| v.get("stable"))
                                    .and_then(|v| v.as_str())
                                    .map(|v| v.to_string());
                                out.push(ToolSearchResult {
                                    provider: "brew".to_string(),
                                    package: name.to_string(),
                                    version,
                                    description: desc.to_string(),
                                    homepage: formula
                                        .get("homepage")
                                        .and_then(|v| v.as_str())
                                        .map(|v| v.to_string()),
                                    score: None,
                                    downloads: None,
                                    installed_path,
                                    install_cmd: Some(format!("brew install {name}")),
                                });
                                added += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // PyPI: exact package lookup.
    if include("pypi") {
        let pypi_url = format!("https://pypi.org/pypi/{}/json", query);
        if let Ok(resp) = client.get(&pypi_url).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    let info = body.get("info").cloned().unwrap_or(Value::Null);
                    let name = info
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(query)
                        .to_string();
                    let installed_path = which_binary(&name).or_else(|| which_binary(exe_guess));
                    out.push(ToolSearchResult {
                        provider: "pypi".to_string(),
                        package: name.clone(),
                        version: info
                            .get("version")
                            .and_then(|v| v.as_str())
                            .map(|v| v.to_string()),
                        description: info
                            .get("summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        homepage: info
                            .get("home_page")
                            .and_then(|v| v.as_str())
                            .map(|v| v.to_string()),
                        score: None,
                        downloads: None,
                        installed_path,
                        install_cmd: Some(format!("pip install {name}")),
                    });
                }
            }
        }
    }

    Ok(out)
}

/// Scan $PATH for useful CLI tools and return basic metadata.
pub fn discover_path_tools() -> Vec<DiscoveredTool> {
    // Curated list of common useful CLI tools the LLM might want to invoke,
    // grouped by category.
    let candidates: &[(&str, &str, &str)] = &[
        // (binary, description, category)
        ("exa", "Modern replacement for ls", "files"),
        ("eza", "Modern replacement for ls (eza fork)", "files"),
        ("bat", "cat with syntax highlighting", "files"),
        ("fd", "Fast alternative to find", "files"),
        ("fzf", "Fuzzy finder", "search"),
        ("rg", "ripgrep: fast regex search", "search"),
        ("ag", "The Silver Searcher", "search"),
        ("jq", "JSON processor", "data"),
        ("yq", "YAML/JSON/XML processor", "data"),
        ("fx", "Interactive JSON viewer", "data"),
        ("delta", "Syntax-highlighting diff viewer", "git"),
        ("gh", "GitHub CLI", "git"),
        ("git", "Version control system", "git"),
        ("curl", "HTTP client", "network"),
        ("wget", "HTTP downloader", "network"),
        ("httpie", "User-friendly HTTP client", "network"),
        ("http", "HTTPie CLI", "network"),
        ("dog", "DNS lookup client", "network"),
        ("dig", "DNS lookup", "network"),
        ("python3", "Python interpreter", "runtime"),
        ("node", "Node.js runtime", "runtime"),
        ("bun", "JavaScript runtime & bundler", "runtime"),
        ("deno", "JavaScript/TypeScript runtime", "runtime"),
        ("cargo", "Rust package manager", "runtime"),
        ("ffmpeg", "Audio/video converter", "media"),
        ("imagemagick", "Image manipulation", "media"),
        ("convert", "ImageMagick convert", "media"),
        ("pandoc", "Document converter", "docs"),
        ("pdftotext", "PDF to text extraction", "docs"),
        ("sqlite3", "SQLite CLI", "data"),
        ("psql", "PostgreSQL CLI", "data"),
        ("redis-cli", "Redis CLI", "data"),
        ("docker", "Container runtime", "infra"),
        ("kubectl", "Kubernetes CLI", "infra"),
        ("terraform", "Infrastructure as code", "infra"),
        ("aws", "AWS CLI", "cloud"),
        ("gcloud", "Google Cloud CLI", "cloud"),
        ("az", "Azure CLI", "cloud"),
    ];

    let mut found = Vec::new();
    for (bin, desc, category) in candidates {
        if let Some(path) = which_binary(bin) {
            let version = probe_version(bin);
            found.push(DiscoveredTool {
                name: bin.to_string(),
                path,
                description: Some(desc.to_string()),
                version,
                category: category.to_string(),
            });
        }
    }
    found
}

pub fn import_tool(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
    request: ToolImportRequest,
) -> Result<ToolRecord, String> {
    let provider = request.provider.trim().to_ascii_lowercase();
    if provider.is_empty() {
        return Err("provider is required".to_string());
    }
    let requested_scope = request.scope.as_deref().unwrap_or("project");
    let scope = ToolScope::parse(requested_scope)
        .ok_or_else(|| format!("invalid scope '{requested_scope}' (expected project/global)"))?;
    let tools_dir = resolve_scope_home(project_home.clone(), global_home.clone(), scope)?;
    std::fs::create_dir_all(&tools_dir)
        .map_err(|e| format!("failed to create {}: {e}", tools_dir.display()))?;

    let default_name = if !request.package.trim().is_empty() {
        request.package.clone()
    } else {
        request
            .local_path
            .clone()
            .unwrap_or_else(|| "tool".to_string())
    };
    let tool_slug = slugify_tool_name(&default_name, "tool");
    let tool_dir = tools_dir.join(&tool_slug);
    std::fs::create_dir_all(&tool_dir)
        .map_err(|e| format!("failed to create {}: {e}", tool_dir.display()))?;

    let mut manifest = ToolManifest {
        name: tool_slug.clone(),
        description: format!("Imported {} tool {}", provider, request.package),
        runtime: ToolRuntime::Shell,
        entrypoint: "run.sh".to_string(),
        command: None,
        timeout_sec: request.timeout_sec.unwrap_or(60).max(1),
        safety: request.safety.unwrap_or_else(|| "readonly".to_string()),
        tags: vec![provider.clone(), "imported".to_string()],
        args: Vec::new(),
        working_dir: None,
    };
    let mut entry_content: Option<String> = None;
    let mut state = ToolState {
        source_kind: provider.clone(),
        source_ref: request.package.clone(),
        source_version: request.version.clone(),
        imported_at: Some(Utc::now().to_rfc3339()),
        approval_required: request.approval_required.unwrap_or(true),
        approved: false,
        approved_by: None,
        approved_at: None,
        hash_pinning: request.hash_pinning.unwrap_or(true),
        pinned_sha256: None,
        auto_skill_trigger: None,
        notes: Vec::new(),
    };

    match provider.as_str() {
        "local" => {
            let src_raw = request
                .local_path
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| "local_path is required for provider=local".to_string())?;
            let src_path = PathBuf::from(src_raw);
            if !src_path.exists() || !src_path.is_file() {
                return Err(format!("local_path '{}' is not a file", src_path.display()));
            }
            let file_name = src_path
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or_else(|| "local file name is invalid unicode".to_string())?;
            let dst_rel = format!("bin/{file_name}");
            let dst = tool_dir.join(&dst_rel);
            copy_with_exec_bits(&src_path, &dst)?;
            manifest.runtime = provider_runtime_from_name(file_name);
            manifest.entrypoint = dst_rel;
            manifest.description = format!("Imported local tool from {}", src_path.display());
            state.source_ref = src_path.display().to_string();
            state.pinned_sha256 = if state.hash_pinning {
                sha256_file(&dst).ok()
            } else {
                None
            };
        }
        "crates" => {
            let package = request.package.trim();
            if package.is_empty() {
                return Err("package is required for provider=crates".to_string());
            }
            let executable = request
                .executable
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(package);
            entry_content = Some(provider_script_crates(
                package,
                request.version.as_deref(),
                executable.trim(),
            ));
            manifest.description = format!("Cargo tool wrapper for {}", package);
            state.source_ref = package.to_string();
        }
        "npm" => {
            let package = request.package.trim();
            if package.is_empty() {
                return Err("package is required for provider=npm".to_string());
            }
            let executable = request
                .executable
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(package);
            entry_content = Some(provider_script_npm(
                package,
                request.version.as_deref(),
                executable.trim(),
            ));
            manifest.description = format!("npm tool wrapper for {}", package);
            state.source_ref = package.to_string();
        }
        "github" => {
            let repo = request
                .repo_url
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    if request.package.trim().is_empty() {
                        None
                    } else {
                        Some(request.package.trim())
                    }
                })
                .ok_or_else(|| "repo_url or package is required for provider=github".to_string())?;
            let entrypoint = request
                .entrypoint
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("run.sh");
            entry_content = Some(provider_script_github(repo, entrypoint));
            manifest.description = format!("GitHub tool wrapper for {}", repo);
            state.source_ref = repo.to_string();
        }
        // Register a binary that is already installed on $PATH.
        // No file copying, no quarantine — the binary is already trusted.
        "path" | "system" => {
            let binary = request
                .executable
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    if !request.package.trim().is_empty() {
                        Some(request.package.trim())
                    } else {
                        None
                    }
                })
                .ok_or_else(|| "package or executable is required for provider=path".to_string())?;

            let resolved = which_binary(binary)
                .ok_or_else(|| format!("binary '{}' not found in $PATH", binary))?;

            manifest.command = Some(binary.to_string());
            manifest.entrypoint = String::new();
            manifest.runtime = provider_runtime_from_name(binary);
            manifest.description = format!("System-installed CLI tool: {}", binary);
            state.source_kind = "path".to_string();
            state.source_ref = resolved.clone();
            // Already on the system — approve immediately, no quarantine.
            state.approval_required = false;
            state.approved = true;
            state.approved_at = Some(Utc::now().to_rfc3339());
            state.approved_by = Some("system-path".to_string());
            state.hash_pinning = false;
        }
        other => {
            return Err(format!(
                "unsupported provider '{}'; expected local|path|crates|npm|github",
                other
            ))
        }
    }

    let entry_path = tool_dir.join(&manifest.entrypoint);
    if let Some(content) = entry_content {
        if let Some(parent) = entry_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
        std::fs::write(&entry_path, content)
            .map_err(|e| format!("failed to write {}: {e}", entry_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&entry_path)
                .map_err(|e| format!("failed to stat {}: {e}", entry_path.display()))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&entry_path, perms)
                .map_err(|e| format!("failed to chmod {}: {e}", entry_path.display()))?;
        }
        if state.hash_pinning {
            state.pinned_sha256 = sha256_file(&entry_path).ok();
        }
    }

    let manifest_path = tool_dir.join("TOOL.yaml");
    let yaml = serde_yaml::to_string(&manifest)
        .map_err(|e| format!("failed to serialize manifest: {e}"))?;
    std::fs::write(&manifest_path, yaml)
        .map_err(|e| format!("failed to write {}: {e}", manifest_path.display()))?;
    save_state(&tool_dir, &state)?;

    get_tool_by_name(project_home, global_home, &manifest.name)
        .and_then(|opt| opt.ok_or_else(|| "imported tool not found after write".to_string()))
}

pub fn approve_tool(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
    name: &str,
    scope: Option<ToolScope>,
    approved_by: Option<String>,
) -> Result<ToolRecord, String> {
    let name_key = name.trim().to_ascii_lowercase();
    if name_key.is_empty() {
        return Err("tool name cannot be empty".to_string());
    }
    let all_scopes = match scope {
        Some(one) => vec![one],
        None => vec![ToolScope::Project, ToolScope::Global],
    };
    let mut updated = false;
    for s in all_scopes {
        let tools_dir = match resolve_scope_home(project_home.clone(), global_home.clone(), s) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !tools_dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&tools_dir)
            .map_err(|e| format!("failed to read {}: {e}", tools_dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(manifest_path) = find_manifest_path(&path) else {
                continue;
            };
            let Ok(manifest) = parse_manifest(&manifest_path) else {
                continue;
            };
            if manifest.name.to_ascii_lowercase() != name_key {
                continue;
            }
            let mut state = load_state(&path);
            state.approval_required = true;
            state.approved = true;
            state.approved_at = Some(Utc::now().to_rfc3339());
            state.approved_by = Some(
                approved_by
                    .clone()
                    .unwrap_or_else(|| "dashboard-user".to_string()),
            );
            let entry_path = path.join(&manifest.entrypoint);
            if state.hash_pinning && entry_path.is_file() {
                state.pinned_sha256 = sha256_file(&entry_path).ok();
            }
            save_state(&path, &state)?;
            updated = true;
            break;
        }
        if updated {
            break;
        }
    }

    if !updated {
        return Err("tool not found in selected scope".to_string());
    }
    get_tool_by_name(project_home, global_home, name)
        .and_then(|opt| opt.ok_or_else(|| "approved tool not found".to_string()))
}

pub fn set_auto_skill_trigger(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
    name: &str,
    scope: ToolScope,
    trigger: &str,
) -> Result<(), String> {
    let tools_dir = resolve_scope_home(project_home, global_home, scope)?;
    if !tools_dir.exists() {
        return Err("tools directory not found".to_string());
    }
    let key = name.trim().to_ascii_lowercase();
    for entry in std::fs::read_dir(&tools_dir)
        .map_err(|e| format!("failed to read {}: {e}", tools_dir.display()))?
        .flatten()
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(manifest_path) = find_manifest_path(&path) else {
            continue;
        };
        let Ok(manifest) = parse_manifest(&manifest_path) else {
            continue;
        };
        if manifest.name.to_ascii_lowercase() != key {
            continue;
        }
        let mut state = load_state(&path);
        state.auto_skill_trigger = Some(trigger.to_string());
        save_state(&path, &state)?;
        return Ok(());
    }
    Err("tool not found for skill mapping".to_string())
}

pub async fn test_tool(
    project_home: Option<PathBuf>,
    global_home: PathBuf,
    name: &str,
    args: Option<Value>,
) -> Result<ToolTestResult, String> {
    let descriptor = resolve_tool_descriptor(project_home, global_home, name)?
        .ok_or_else(|| format!("tool '{}' not found", name.trim()))?;

    if descriptor.state.approval_required && !descriptor.state.approved {
        return Err(format!(
            "tool '{}' is in quarantine; approve it before execution",
            descriptor.manifest.name
        ));
    }
    if descriptor.state.hash_pinning {
        let current_hash = if descriptor.entry_path.is_file() {
            sha256_file(&descriptor.entry_path)?
        } else {
            String::new()
        };
        let expected = descriptor.state.pinned_sha256.clone().unwrap_or_default();
        if expected.is_empty() || !expected.eq_ignore_ascii_case(&current_hash) {
            return Err(format!(
                "tool '{}' hash mismatch; re-approve to refresh pin",
                descriptor.manifest.name
            ));
        }
    }

    let args = render_manifest_args(&descriptor.manifest, args)?;

    let mut command_line: Vec<String> = Vec::new();
    let mut command = if let Some(cmd) = &descriptor.manifest.command {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return Err("manifest command cannot be empty".to_string());
        }
        command_line.push(trimmed.to_string());
        Command::new(trimmed)
    } else {
        if !descriptor.entry_path.exists() {
            return Err(format!(
                "tool entrypoint '{}' does not exist",
                descriptor.entry_path.display()
            ));
        }

        match descriptor.manifest.runtime {
            ToolRuntime::Shell => {
                command_line.push("sh".to_string());
                command_line.push(descriptor.entry_path.display().to_string());
                let mut cmd = Command::new("sh");
                cmd.arg(descriptor.entry_path.as_os_str());
                cmd
            }
            ToolRuntime::Python => {
                command_line.push("python3".to_string());
                command_line.push(descriptor.entry_path.display().to_string());
                let mut cmd = Command::new("python3");
                cmd.arg(descriptor.entry_path.as_os_str());
                cmd
            }
            ToolRuntime::Node => {
                command_line.push("node".to_string());
                command_line.push(descriptor.entry_path.display().to_string());
                let mut cmd = Command::new("node");
                cmd.arg(descriptor.entry_path.as_os_str());
                cmd
            }
            ToolRuntime::Binary => {
                command_line.push(descriptor.entry_path.display().to_string());
                Command::new(&descriptor.entry_path)
            }
        }
    };

    for arg in &args {
        command_line.push(arg.clone());
        command.arg(arg);
    }

    let working_dir = descriptor
        .manifest
        .working_dir
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| descriptor.tool_root.clone());
    command.current_dir(working_dir);

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let started_at = std::time::Instant::now();
    let timeout = Duration::from_secs(descriptor.manifest.timeout_sec.max(1));

    command.kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|e| format!("failed to spawn tool '{}': {e}", descriptor.manifest.name))?;

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => result
            .map_err(|e| format!("failed to await tool '{}': {e}", descriptor.manifest.name))?,
        Err(_) => {
            return Ok(ToolTestResult {
                ok: false,
                exit_code: None,
                timed_out: true,
                duration_ms: started_at.elapsed().as_millis(),
                stdout: String::new(),
                stderr: format!(
                    "tool '{}' timed out after {}s",
                    descriptor.manifest.name, descriptor.manifest.timeout_sec
                ),
                command: command_line,
            })
        }
    };

    let duration_ms = started_at.elapsed().as_millis();
    let exit_code = output.status.code();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(ToolTestResult {
        ok: output.status.success(),
        exit_code,
        timed_out: false,
        duration_ms,
        stdout,
        stderr,
        command: command_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_normalization_rejects_invalid_timeout() {
        let mut manifest = ToolManifest {
            name: "demo".to_string(),
            description: "demo".to_string(),
            runtime: ToolRuntime::Shell,
            entrypoint: "run.sh".to_string(),
            command: None,
            timeout_sec: 7200,
            safety: "readonly".to_string(),
            tags: vec![],
            args: vec![],
            working_dir: None,
        };
        let err = normalize_manifest(&mut manifest).unwrap_err();
        assert!(err.contains("timeout_sec"));
    }

    #[test]
    fn render_manifest_args_validates_required_fields() {
        let manifest = ToolManifest {
            name: "demo".to_string(),
            description: "demo".to_string(),
            runtime: ToolRuntime::Shell,
            entrypoint: "run.sh".to_string(),
            command: None,
            timeout_sec: 60,
            safety: "readonly".to_string(),
            tags: vec![],
            args: vec![ToolArgSpec {
                name: "target".to_string(),
                description: "target path".to_string(),
                arg_type: "string".to_string(),
                required: true,
            }],
            working_dir: None,
        };

        let err = render_manifest_args(&manifest, Some(serde_json::json!({}))).unwrap_err();
        assert!(err.contains("missing required arg"));
    }

    #[test]
    fn list_tools_applies_project_precedence() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();

        let global_tools = global.path().join("tools").join("lint");
        std::fs::create_dir_all(&global_tools).unwrap();
        std::fs::write(
            global_tools.join("TOOL.yaml"),
            "name: lint\ndescription: global\nruntime: shell\nentrypoint: run.sh\ntimeout_sec: 30\n",
        )
        .unwrap();
        std::fs::write(global_tools.join("run.sh"), "echo global\n").unwrap();

        let project_tools = project.path().join("tools").join("lint");
        std::fs::create_dir_all(&project_tools).unwrap();
        std::fs::write(
            project_tools.join("TOOL.yaml"),
            "name: lint\ndescription: project\nruntime: shell\nentrypoint: run.sh\ntimeout_sec: 30\n",
        )
        .unwrap();
        std::fs::write(project_tools.join("run.sh"), "echo project\n").unwrap();

        let tools = list_tools(
            Some(project.path().to_path_buf()),
            global.path().to_path_buf(),
        )
        .unwrap();

        let lint = tools.iter().find(|tool| tool.name == "lint").unwrap();
        assert_eq!(lint.source, "project");
        assert!(lint.has_collisions);
        assert!(lint.shadowed_sources.iter().any(|scope| scope == "global"));
    }

    #[tokio::test]
    async fn test_tool_blocks_quarantined_tool() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let tool_dir = project.path().join("tools").join("danger");
        std::fs::create_dir_all(&tool_dir).unwrap();
        std::fs::write(
            tool_dir.join("TOOL.yaml"),
            "name: danger\ndescription: test\nruntime: shell\nentrypoint: run.sh\ntimeout_sec: 5\n",
        )
        .unwrap();
        std::fs::write(tool_dir.join("run.sh"), "#!/usr/bin/env sh\necho ok\n").unwrap();
        std::fs::write(
            tool_dir.join(TOOL_STATE_FILE),
            r#"{
  "source_kind": "local",
  "source_ref": "/tmp/danger",
  "approval_required": true,
  "approved": false,
  "hash_pinning": false
}"#,
        )
        .unwrap();

        let err = test_tool(
            Some(project.path().to_path_buf()),
            global.path().to_path_buf(),
            "danger",
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("quarantine"));
    }

    #[test]
    fn import_local_tool_defaults_to_quarantine_and_hash_pin() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let src = project.path().join("source-tool.sh");
        std::fs::write(&src, "#!/usr/bin/env sh\necho local\n").unwrap();
        let record = import_tool(
            Some(project.path().to_path_buf()),
            global.path().to_path_buf(),
            ToolImportRequest {
                provider: "local".to_string(),
                package: "local-tool".to_string(),
                scope: Some("project".to_string()),
                version: None,
                executable: None,
                local_path: Some(src.display().to_string()),
                repo_url: None,
                entrypoint: None,
                safety: None,
                timeout_sec: Some(30),
                approval_required: None,
                hash_pinning: None,
            },
        )
        .unwrap();
        assert_eq!(record.source_kind, "local");
        assert!(!record.approved);
        assert!(record.approval_required);
        assert!(record.hash_pinning);
        assert!(record.pinned_sha256.is_some());
    }
}
