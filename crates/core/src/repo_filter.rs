use std::path::{Component, Path};

const DEFAULT_MAX_GRAPH_FILE_BYTES: u64 = 1_000_000;
const DEFAULT_MAX_GRAPH_JSON_BYTES: u64 = 50_000_000;
const DEFAULT_MAX_PROMPT_FILE_BYTES: u64 = 512_000;
const DEFAULT_REPO_BRAIN_MEMORY_KEY_LIMIT: usize = 64;

pub fn max_graph_file_bytes() -> u64 {
    env_u64(
        "AGENT007_REPO_GRAPH_MAX_FILE_BYTES",
        DEFAULT_MAX_GRAPH_FILE_BYTES,
    )
}

pub fn max_prompt_file_bytes() -> u64 {
    env_u64(
        "AGENT007_PROMPT_MAX_FILE_BYTES",
        DEFAULT_MAX_PROMPT_FILE_BYTES,
    )
}

pub fn max_graph_json_bytes() -> u64 {
    env_u64(
        "AGENT007_REPO_GRAPH_MAX_JSON_BYTES",
        DEFAULT_MAX_GRAPH_JSON_BYTES,
    )
}

pub fn repo_brain_memory_key_limit() -> usize {
    env_usize(
        "AGENT007_REPO_BRAIN_MAX_MEMORY_KEYS",
        DEFAULT_REPO_BRAIN_MEMORY_KEY_LIMIT,
    )
}

pub fn data_file_symbol_indexing_enabled() -> bool {
    matches!(
        std::env::var("AGENT007_REPO_GRAPH_INDEX_DATA_FILES")
            .ok()
            .as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("on")
    )
}

pub fn should_skip_repo_path(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(value) => value
            .to_str()
            .map(|name| should_skip_dir_name(name) || should_skip_file_name(name))
            .unwrap_or(false),
        _ => false,
    })
}

pub fn should_skip_prompt_path(path: &Path) -> bool {
    should_skip_repo_path(path)
        || path
            .file_name()
            .and_then(|value| value.to_str())
            .map(is_sensitive_file_name)
            .unwrap_or(false)
}

pub fn should_skip_dir_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "target"
            | "node_modules"
            | ".venv"
            | "venv"
            | ".idea"
            | ".zed"
            | ".vscode"
            | ".cursor"
            | ".codex"
            | ".claude"
            | ".claude-flow"
            | "dist"
            | "build"
            | ".next"
            | "coverage"
            | "out"
            | ".cache"
            | "tmp"
            | "temp"
    ) || name.starts_with(".agent007")
}

pub fn should_skip_file_name(name: &str) -> bool {
    is_lockfile_name(name)
        || is_generated_agent_guidance_file(name)
        || is_runtime_artifact_file_name(name)
        || is_sensitive_file_name(name)
        || name.ends_with(".min.js")
        || name.ends_with(".map")
        || name.ends_with(".agent007.bak")
        || name.contains(".agent007.bak.")
}

pub fn is_generated_agent_guidance_file(name: &str) -> bool {
    matches!(name, "AGENTS.agent007.generated.md")
        || name.starts_with("AGENTS.agent007.generated.md.")
}

pub fn is_lockfile_name(name: &str) -> bool {
    matches!(
        name,
        "package-lock.json"
            | "npm-shrinkwrap.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lockb"
            | "Cargo.lock"
            | "composer.lock"
            | "poetry.lock"
            | "Pipfile.lock"
            | "Gemfile.lock"
            | "go.sum"
    )
}

pub fn is_runtime_artifact_file_name(name: &str) -> bool {
    matches!(
        name,
        "repo_graph_v1.json"
            | "repo_graph_dirty_paths.json"
            | "repo_intelligence_readiness_v1.json"
            | "context-bundle.json"
            | "workflow-state.json"
            | "workflow-request.json"
            | "token-summary.json"
            | "run-scorecard.json"
            | "messages.json"
            | "events.jsonl"
            | "meta.json"
            | ".mcp.json"
            | ".rules"
            | ".DS_Store"
    )
}

pub fn is_sensitive_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let path = Path::new(&lower);
    let extension = path.extension().and_then(|value| value.to_str());
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(lower.as_str());
    let sensitive_data_ext = matches!(
        extension,
        None | Some("env" | "json" | "yaml" | "yml" | "toml" | "ini" | "conf" | "txt")
    );
    let sensitive_stem = stem
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|segment| {
            matches!(
                segment,
                "secret" | "secrets" | "credential" | "credentials" | "token" | "tokens" | "apikey"
            )
        });
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
        || lower == "id_rsa"
        || lower == "id_dsa"
        || lower == "credentials.json"
        || lower == "secrets.json"
        || lower.contains("api_key")
        || (sensitive_data_ext && sensitive_stem)
}

pub fn file_is_within_graph_budget(path: &Path) -> bool {
    file_is_within_budget(path, max_graph_file_bytes())
}

pub fn file_is_within_prompt_budget(path: &Path) -> bool {
    file_is_within_budget(path, max_prompt_file_bytes())
}

pub fn graph_json_is_within_load_budget(path: &Path) -> bool {
    file_is_within_budget(path, max_graph_json_bytes())
}

fn file_is_within_budget(path: &Path, max_bytes: u64) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.len() <= max_bytes,
        Ok(_) => true,
        Err(_) => true,
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_filter_skips_secret_data_not_token_code() {
        assert!(should_skip_file_name(".env.local"));
        assert!(should_skip_file_name("credentials.json"));
        assert!(should_skip_file_name("openai_token.txt"));
        assert!(should_skip_file_name("private-key.pem"));
        assert!(!should_skip_file_name("tokenizer.rs"));
        assert!(!should_skip_file_name("auth_token.rs"));
        assert!(!should_skip_file_name("api.yaml"));
    }

    #[test]
    fn repo_filter_skips_agent007_runtime_and_generated_guidance() {
        assert!(should_skip_repo_path(Path::new(
            ".agent007.bak.20260503-142745/sessions/context-bundle.json"
        )));
        assert!(should_skip_repo_path(Path::new(
            ".agent007/runtime/repo_graph_v1.json"
        )));
        assert!(should_skip_repo_path(Path::new(
            "AGENTS.agent007.generated.md.agent007.bak"
        )));
        assert!(should_skip_repo_path(Path::new(
            ".codex/sessions/context-bundle.json"
        )));
        assert!(should_skip_repo_path(Path::new(
            ".claude-flow/state/events.jsonl"
        )));
        assert!(!should_skip_repo_path(Path::new("src/lib.rs")));
    }
}
