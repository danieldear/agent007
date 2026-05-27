use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::repo_graph::{
    build_and_save_graph, default_graph_path_for_root, graph_status, RepoGraphStatus,
};
use crate::tree_sitter_support::support_summary_for_languages;

const READINESS_VERSION: u32 = 1;
const READINESS_FILENAME: &str = "repo_intelligence_readiness_v1.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepoIntelligenceState {
    EmptyRepo,
    BaselineOnly,
    EnrichmentAvailable,
    EnrichmentActive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum LanguageKind {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
    C,
    Cpp,
    Java,
    Kotlin,
    Html,
    Vue,
    Xml,
    Json,
    Yaml,
}

impl LanguageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Go => "go",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Java => "java",
            Self::Kotlin => "kotlin",
            Self::Html => "html",
            Self::Vue => "vue",
            Self::Xml => "xml",
            Self::Json => "json",
            Self::Yaml => "yaml",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRecommendation {
    pub id: String,
    pub kind: String,
    pub language: String,
    pub title: String,
    pub command: String,
    pub can_run: bool,
    pub installed: bool,
    pub configured: bool,
    pub active: bool,
    pub requires_approval: bool,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerReadiness {
    pub language: String,
    pub server_key: String,
    pub command: String,
    pub installed: bool,
    pub configured: bool,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_recommendation: Option<InstallRecommendation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageReadiness {
    pub language: String,
    pub file_count: usize,
    pub manifest_count: usize,
    pub sample_paths: Vec<String>,
    pub signals: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<LspServerReadiness>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeSitterReadiness {
    pub wired: bool,
    pub available: bool,
    pub installable: bool,
    pub status: String,
    pub note: String,
    pub supported_languages: Vec<String>,
    pub active_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIntelligenceReadiness {
    pub version: u32,
    pub root: String,
    pub generated_at: String,
    pub state: RepoIntelligenceState,
    pub baseline_ready: bool,
    pub graph: RepoGraphStatus,
    pub languages: Vec<LanguageReadiness>,
    pub tree_sitter: TreeSitterReadiness,
    pub recommendations: Vec<InstallRecommendation>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepoGraphEnsureAction {
    Skipped,
    EmptyRepo,
    Built,
    Refreshed,
    Fresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGraphEnsureResult {
    pub action: RepoGraphEnsureAction,
    pub matched_reason: String,
    pub readiness: RepoIntelligenceReadiness,
}

#[derive(Debug, Clone, Default)]
pub struct RepoIntelligenceOptions {
    pub lsp_enabled: bool,
    pub configured_servers: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct DetectedLanguageAccumulator {
    file_count: usize,
    manifest_count: usize,
    sample_paths: Vec<String>,
    signals: BTreeSet<String>,
}

pub fn readiness_path_for_root(root: &Path) -> PathBuf {
    default_graph_path_for_root(root)
        .parent()
        .unwrap_or(root)
        .join(READINESS_FILENAME)
}

pub fn detect_repo_intelligence_readiness(
    root: &Path,
    options: &RepoIntelligenceOptions,
) -> Result<RepoIntelligenceReadiness, CoreError> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let graph_path = default_graph_path_for_root(&root);
    let graph = graph_status(&graph_path);
    let languages = detect_languages(&root)?;
    let mut recommendations = Vec::new();
    let mut enriched_languages = Vec::new();
    let mut has_installed_lsp = false;
    let mut has_active_lsp = false;

    for (kind, acc) in languages {
        let language = kind.as_str().to_string();
        let default_server = default_lsp_server_for(&kind);
        let configured_command = options.configured_servers.get(&language).cloned();
        let server_command = configured_command
            .clone()
            .or_else(|| default_server.map(|s| s.0.to_string()));
        let configured = configured_command.is_some();
        let lsp = server_command.map(|command| {
            let server_key = default_server.map(|s| s.0).unwrap_or(command.as_str());
            let installed = binary_on_path(first_command_token(&command));
            let active = options.lsp_enabled && installed;
            if installed {
                has_installed_lsp = true;
            }
            if active {
                has_active_lsp = true;
            }
            let install_recommendation = if !installed {
                default_install_recommendation(&kind, configured, active)
            } else {
                None
            };
            if let Some(rec) = &install_recommendation {
                recommendations.push(rec.clone());
            }
            LspServerReadiness {
                language: language.clone(),
                server_key: server_key.to_string(),
                command,
                installed,
                configured,
                active,
                install_recommendation,
            }
        });

        enriched_languages.push(LanguageReadiness {
            language,
            file_count: acc.file_count,
            manifest_count: acc.manifest_count,
            sample_paths: acc.sample_paths,
            signals: acc.signals.into_iter().collect(),
            lsp,
        });
    }

    let detected_language_kinds: Vec<LanguageKind> = enriched_languages
        .iter()
        .filter_map(|lang| language_kind_from_str(&lang.language))
        .collect();
    let tree_sitter = tree_sitter_readiness_for_languages(&detected_language_kinds);
    let has_tree_sitter = !tree_sitter.active_languages.is_empty();
    let has_tree_sitter_available = !tree_sitter.supported_languages.is_empty();

    let state = if enriched_languages.is_empty() {
        RepoIntelligenceState::EmptyRepo
    } else if has_active_lsp || has_tree_sitter {
        RepoIntelligenceState::EnrichmentActive
    } else if has_installed_lsp || has_tree_sitter_available {
        RepoIntelligenceState::EnrichmentAvailable
    } else {
        RepoIntelligenceState::BaselineOnly
    };

    let mut notes = Vec::new();
    match state {
        RepoIntelligenceState::EmptyRepo => notes.push(
            "Baseline graph runtime is ready; semantic enrichment will activate when source files or manifests appear.".to_string(),
        ),
        RepoIntelligenceState::BaselineOnly => notes.push(
            "Source files were detected, but no semantic enrichers are currently installed or configured.".to_string(),
        ),
        RepoIntelligenceState::EnrichmentAvailable => notes.push(
            "Semantic enrichers are available on this machine or in the current build, but agent007 is still operating in baseline structural mode for at least one detected language.".to_string(),
        ),
        RepoIntelligenceState::EnrichmentActive => notes.push(
            "Semantic enrichment is active for at least one detected language.".to_string(),
        ),
    }
    if !graph.exists {
        notes.push("Repo graph artifact is missing; build or refresh the graph to enable structural queries.".to_string());
    } else if graph.stale {
        notes.push("Repo graph artifact is stale; refresh it to align structural results with the current workspace.".to_string());
    }

    Ok(RepoIntelligenceReadiness {
        version: READINESS_VERSION,
        root: root.display().to_string(),
        generated_at: Utc::now().to_rfc3339(),
        state,
        baseline_ready: true,
        graph,
        languages: enriched_languages,
        tree_sitter,
        recommendations,
        notes,
    })
}

pub fn write_repo_intelligence_readiness(
    root: &Path,
    path: Option<&Path>,
    options: &RepoIntelligenceOptions,
) -> Result<RepoIntelligenceReadiness, CoreError> {
    let readiness = detect_repo_intelligence_readiness(root, options)?;
    let out_path = path
        .map(PathBuf::from)
        .unwrap_or_else(|| readiness_path_for_root(root));
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
    }
    fs::write(&out_path, serde_json::to_string_pretty(&readiness)?)
        .map_err(|e| CoreError::io(&out_path, e))?;
    Ok(readiness)
}

pub fn load_repo_intelligence_readiness(
    path: &Path,
) -> Result<RepoIntelligenceReadiness, CoreError> {
    let raw = fs::read_to_string(path).map_err(|e| CoreError::io(path, e))?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn task_requests_repo_graph(task: &str) -> Option<&'static str> {
    let normalized = task.to_ascii_lowercase();
    let checks = [
        ("/dev-pr-review", "skill:/dev-pr-review"),
        ("/meta-analyze-codebase", "skill:/meta-analyze-codebase"),
        ("/code-review", "skill:/code-review"),
        ("/code-security-audit", "skill:/code-security-audit"),
        ("/dev-debug", "skill:/dev-debug"),
        ("/workflow:code-review", "workflow:code-review"),
        ("analyze the code", "phrase:analyze the code"),
        ("analyze code", "phrase:analyze code"),
        ("analyze the codebase", "phrase:analyze the codebase"),
        ("analyze codebase", "phrase:analyze codebase"),
        ("review the code", "phrase:review the code"),
        ("review code", "phrase:review code"),
        ("code review", "phrase:code review"),
        ("audit the code", "phrase:audit the code"),
        ("inspect the codebase", "phrase:inspect the codebase"),
    ];
    checks
        .iter()
        .find_map(|(needle, reason)| normalized.contains(needle).then_some(*reason))
}

pub fn ensure_repo_graph_ready_for_task(
    root: &Path,
    task: &str,
    options: &RepoIntelligenceOptions,
) -> Result<Option<RepoGraphEnsureResult>, CoreError> {
    let Some(reason) = task_requests_repo_graph(task) else {
        return Ok(None);
    };
    Ok(Some(ensure_repo_graph_ready(root, reason, options)?))
}

pub fn ensure_repo_graph_ready_for_trigger(
    root: &Path,
    trigger: &str,
    options: &RepoIntelligenceOptions,
) -> Result<Option<RepoGraphEnsureResult>, CoreError> {
    let normalized = trigger.trim().to_ascii_lowercase();
    let reason = match normalized.as_str() {
        "/dev-pr-review" => Some("skill:/dev-pr-review"),
        "/meta-analyze-codebase" => Some("skill:/meta-analyze-codebase"),
        "/code-security-audit" => Some("skill:/code-security-audit"),
        "/dev-debug" => Some("skill:/dev-debug"),
        _ => None,
    };
    Ok(reason
        .map(|r| ensure_repo_graph_ready(root, r, options))
        .transpose()?)
}

fn ensure_repo_graph_ready(
    root: &Path,
    matched_reason: &str,
    options: &RepoIntelligenceOptions,
) -> Result<RepoGraphEnsureResult, CoreError> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut readiness = write_repo_intelligence_readiness(&root, None, options)?;

    let action = if readiness.state == RepoIntelligenceState::EmptyRepo {
        RepoGraphEnsureAction::EmptyRepo
    } else if !readiness.graph.exists {
        let _ = build_and_save_graph(&root, None)?;
        readiness = write_repo_intelligence_readiness(&root, None, options)?;
        RepoGraphEnsureAction::Built
    } else if readiness.graph.stale {
        let _ = build_and_save_graph(&root, None)?;
        readiness = write_repo_intelligence_readiness(&root, None, options)?;
        RepoGraphEnsureAction::Refreshed
    } else {
        RepoGraphEnsureAction::Fresh
    };

    Ok(RepoGraphEnsureResult {
        action,
        matched_reason: matched_reason.to_string(),
        readiness,
    })
}

fn detect_languages(
    root: &Path,
) -> Result<BTreeMap<LanguageKind, DetectedLanguageAccumulator>, CoreError> {
    let mut found: BTreeMap<LanguageKind, DetectedLanguageAccumulator> = BTreeMap::new();
    walk(root, root, &mut found)?;
    Ok(found)
}

fn walk(
    root: &Path,
    dir: &Path,
    found: &mut BTreeMap<LanguageKind, DetectedLanguageAccumulator>,
) -> Result<(), CoreError> {
    for entry in fs::read_dir(dir).map_err(|e| CoreError::io(dir, e))? {
        let entry = entry.map_err(|e| CoreError::io(dir, e))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry
            .file_type()
            .map_err(|e| CoreError::io(&path, e))?
            .is_dir()
        {
            if skip_dir(&name) {
                continue;
            }
            walk(root, &path, found)?;
            continue;
        }
        if !entry
            .file_type()
            .map_err(|e| CoreError::io(&path, e))?
            .is_file()
        {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let manifest_kind = manifest_language(&name);
        if let Some(kind) = manifest_kind {
            let acc = found.entry(kind).or_default();
            acc.manifest_count += 1;
            push_unique_sample(&mut acc.sample_paths, rel.clone());
            acc.signals.insert(format!("manifest:{name}"));
        }
        if let Some(kind) = extension_language(&path) {
            let acc = found.entry(kind).or_default();
            acc.file_count += 1;
            push_unique_sample(&mut acc.sample_paths, rel.clone());
            if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                acc.signals.insert(format!("ext:.{ext}"));
            }
        }
    }
    Ok(())
}

fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".agent007"
            | "target"
            | "node_modules"
            | "dist"
            | "build"
            | ".venv"
            | "venv"
            | "coverage"
    )
}

fn manifest_language(name: &str) -> Option<LanguageKind> {
    match name {
        "Cargo.toml" => Some(LanguageKind::Rust),
        "pyproject.toml" | "setup.py" | "requirements.txt" => Some(LanguageKind::Python),
        "package.json" | "tsconfig.json" => Some(LanguageKind::TypeScript),
        "go.mod" => Some(LanguageKind::Go),
        "CMakeLists.txt" | "compile_commands.json" => Some(LanguageKind::Cpp),
        "pom.xml" | "build.gradle" | "build.gradle.kts" => Some(LanguageKind::Java),
        "tailwind.config.js"
        | "tailwind.config.ts"
        | "tailwind.config.cjs"
        | "tailwind.config.mjs" => Some(LanguageKind::Html),
        _ => None,
    }
}

fn extension_language(path: &Path) -> Option<LanguageKind> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
    {
        "rs" => Some(LanguageKind::Rust),
        "py" => Some(LanguageKind::Python),
        "ts" | "tsx" => Some(LanguageKind::TypeScript),
        "js" | "jsx" | "mjs" | "cjs" => Some(LanguageKind::JavaScript),
        "go" => Some(LanguageKind::Go),
        "c" | "h" => Some(LanguageKind::C),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some(LanguageKind::Cpp),
        "java" => Some(LanguageKind::Java),
        "kt" | "kts" => Some(LanguageKind::Kotlin),
        "html" | "htm" => Some(LanguageKind::Html),
        "vue" => Some(LanguageKind::Vue),
        "xml" => Some(LanguageKind::Xml),
        "json" => Some(LanguageKind::Json),
        "yaml" | "yml" => Some(LanguageKind::Yaml),
        _ => None,
    }
}

fn push_unique_sample(samples: &mut Vec<String>, path: String) {
    if samples.iter().any(|p| p == &path) {
        return;
    }
    if samples.len() < 4 {
        samples.push(path);
    }
}

fn first_command_token(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or(command)
}

fn binary_on_path(binary: &str) -> bool {
    if binary.is_empty() {
        return false;
    }
    let candidate = Path::new(binary);
    if candidate.is_absolute() && candidate.exists() {
        return true;
    }
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path_var).any(|dir| {
        let full = dir.join(binary);
        if full.is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{binary}.exe"));
            if exe.is_file() {
                return true;
            }
        }
        false
    })
}

fn default_lsp_server_for(kind: &LanguageKind) -> Option<(&'static str, &'static str)> {
    match kind {
        LanguageKind::Rust => Some(("rust-analyzer", "rust-analyzer")),
        LanguageKind::Python => Some(("pyright", "pyright --stdio")),
        LanguageKind::TypeScript | LanguageKind::JavaScript => Some((
            "typescript-language-server",
            "typescript-language-server --stdio",
        )),
        LanguageKind::Go => Some(("gopls", "gopls")),
        LanguageKind::C | LanguageKind::Cpp => Some(("clangd", "clangd")),
        LanguageKind::Java
        | LanguageKind::Kotlin
        | LanguageKind::Html
        | LanguageKind::Vue
        | LanguageKind::Xml
        | LanguageKind::Json
        | LanguageKind::Yaml => None,
    }
}

fn language_kind_from_str(value: &str) -> Option<LanguageKind> {
    match value {
        "rust" => Some(LanguageKind::Rust),
        "python" => Some(LanguageKind::Python),
        "typescript" => Some(LanguageKind::TypeScript),
        "javascript" => Some(LanguageKind::JavaScript),
        "go" => Some(LanguageKind::Go),
        "c" => Some(LanguageKind::C),
        "cpp" => Some(LanguageKind::Cpp),
        "java" => Some(LanguageKind::Java),
        "kotlin" => Some(LanguageKind::Kotlin),
        "html" => Some(LanguageKind::Html),
        "vue" => Some(LanguageKind::Vue),
        "xml" => Some(LanguageKind::Xml),
        "json" => Some(LanguageKind::Json),
        "yaml" => Some(LanguageKind::Yaml),
        _ => None,
    }
}

fn tree_sitter_readiness_for_languages(languages: &[LanguageKind]) -> TreeSitterReadiness {
    let summary = support_summary_for_languages(languages.iter().cloned());
    let status = if languages.is_empty() {
        "deferred"
    } else if summary.active_languages.is_empty() {
        "unsupported"
    } else if summary.active_languages.len() == languages.len() {
        "active"
    } else {
        "partial"
    };
    let note = if languages.is_empty() {
        "Built-in tree-sitter enrichment is wired and will activate automatically when supported source files appear.".to_string()
    } else if summary.active_languages.is_empty() {
        "No built-in tree-sitter grammar is wired for the currently detected languages in this build; baseline graph + LSP remain available.".to_string()
    } else if summary.active_languages.len() == languages.len() {
        format!(
            "Built-in tree-sitter enrichment is active for all detected supported languages: {}.",
            summary.active_languages.join(", ")
        )
    } else {
        format!(
            "Built-in tree-sitter enrichment is active for {}. Other detected languages continue with the baseline graph and optional LSP until more grammars are added.",
            summary.active_languages.join(", ")
        )
    };
    TreeSitterReadiness {
        wired: summary.wired,
        available: !summary.supported_languages.is_empty(),
        installable: false,
        status: status.to_string(),
        note,
        supported_languages: summary.supported_languages,
        active_languages: summary.active_languages,
    }
}

fn default_install_recommendation(
    kind: &LanguageKind,
    configured: bool,
    active: bool,
) -> Option<InstallRecommendation> {
    let (server_key, title, command) = match kind {
        LanguageKind::Rust => (
            "rust-analyzer",
            "Install rust-analyzer",
            if cfg!(target_os = "macos") {
                "brew install rust-analyzer"
            } else {
                "rustup component add rust-analyzer"
            },
        ),
        LanguageKind::Python => ("pyright", "Install pyright", "npm install -g pyright"),
        LanguageKind::TypeScript | LanguageKind::JavaScript => (
            "typescript-language-server",
            "Install TypeScript language server",
            "npm install -g typescript typescript-language-server",
        ),
        LanguageKind::Go => (
            "gopls",
            "Install gopls",
            "go install golang.org/x/tools/gopls@latest",
        ),
        LanguageKind::C | LanguageKind::Cpp => (
            "clangd",
            "Install clangd",
            if cfg!(target_os = "macos") {
                "brew install llvm"
            } else {
                "sudo apt-get install -y clangd"
            },
        ),
        LanguageKind::Java
        | LanguageKind::Kotlin
        | LanguageKind::Html
        | LanguageKind::Vue
        | LanguageKind::Xml
        | LanguageKind::Json
        | LanguageKind::Yaml => return None,
    };
    Some(InstallRecommendation {
        id: format!("lsp:{}", kind.as_str()),
        kind: "lsp".to_string(),
        language: kind.as_str().to_string(),
        title: title.to_string(),
        command: command.to_string(),
        can_run: !command.contains("sudo ") && binary_on_path(first_command_token(command)),
        installed: false,
        configured,
        active,
        requires_approval: true,
        source: server_key.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_marks_empty_repo() {
        let dir = tempfile::tempdir().unwrap();
        let readiness =
            detect_repo_intelligence_readiness(dir.path(), &RepoIntelligenceOptions::default())
                .unwrap();
        assert_eq!(readiness.state, RepoIntelligenceState::EmptyRepo);
        assert!(readiness.languages.is_empty());
    }

    #[test]
    fn readiness_detects_rust_manifest_and_source() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
        let readiness =
            detect_repo_intelligence_readiness(dir.path(), &RepoIntelligenceOptions::default())
                .unwrap();
        assert_ne!(readiness.state, RepoIntelligenceState::EmptyRepo);
        assert_eq!(readiness.languages[0].language, "rust");
        assert!(readiness.languages[0].manifest_count >= 1);
        assert!(readiness.languages[0].file_count >= 1);
        let rust_lsp = readiness.languages[0].lsp.as_ref().unwrap();
        assert_eq!(rust_lsp.language, "rust");
        assert!(rust_lsp.command.contains("rust-analyzer"));
        assert!(readiness.tree_sitter.wired);
        assert_eq!(readiness.tree_sitter.status, "active");
        assert_eq!(readiness.tree_sitter.active_languages, vec!["rust"]);
    }

    #[test]
    fn readiness_writes_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let out = write_repo_intelligence_readiness(
            dir.path(),
            None,
            &RepoIntelligenceOptions::default(),
        )
        .unwrap();
        let path = readiness_path_for_root(dir.path());
        assert!(path.exists());
        let loaded = load_repo_intelligence_readiness(&path).unwrap();
        assert_eq!(loaded.state, out.state);
    }

    #[test]
    fn task_requests_repo_graph_matches_analysis_inputs() {
        assert_eq!(
            task_requests_repo_graph("/meta-analyze-codebase summarize repo"),
            Some("skill:/meta-analyze-codebase")
        );
        assert_eq!(
            task_requests_repo_graph("please analyze the code before editing"),
            Some("phrase:analyze the code")
        );
        assert_eq!(task_requests_repo_graph("say hello"), None);
    }

    #[test]
    fn ensure_repo_graph_ready_builds_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn demo() {}\n").unwrap();

        let result = ensure_repo_graph_ready_for_task(
            dir.path(),
            "/meta-analyze-codebase summarize repo",
            &RepoIntelligenceOptions::default(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.action, RepoGraphEnsureAction::Built);
        assert!(result.readiness.graph.exists);
    }

    #[test]
    fn ensure_repo_graph_ready_skips_empty_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = ensure_repo_graph_ready_for_task(
            dir.path(),
            "/meta-analyze-codebase summarize repo",
            &RepoIntelligenceOptions::default(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.action, RepoGraphEnsureAction::EmptyRepo);
        assert!(!result.readiness.graph.exists);
    }
}
