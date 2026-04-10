use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CoreConfig {
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
    #[serde(default = "default_task_queue_capacity")]
    pub task_queue_capacity: usize,
}
fn default_max_agents() -> usize {
    8
}
fn default_task_queue_capacity() -> usize {
    256
}
impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            max_agents: default_max_agents(),
            task_queue_capacity: default_task_queue_capacity(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RoutingConfig {
    pub code_completion: Option<String>,
    pub reasoning: Option<String>,
    pub fast_local: Option<String>,
    pub sensitive: Option<String>,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OllamaModelConfig {
    #[serde(default = "default_ollama_url")]
    pub base_url: String,
    #[serde(default = "default_ollama_model")]
    pub default_model: String,
}
fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}
fn default_ollama_model() -> String {
    "llama3".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClaudeModelConfig {
    #[serde(default = "default_claude_model")]
    pub default_model: String,
}
fn default_claude_model() -> String {
    "claude-sonnet-4-6".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodexModelConfig {
    #[serde(default = "default_codex_model")]
    pub default_model: String,
}
fn default_codex_model() -> String {
    "gpt-5.3-codex".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelsConfig {
    #[serde(default = "default_model")]
    pub default: String,
    pub routing: Option<RoutingConfig>,
    pub claude: Option<ClaudeModelConfig>,
    pub codex: Option<CodexModelConfig>,
    pub ollama: Option<OllamaModelConfig>,
}
fn default_model() -> String {
    "claude".to_string()
}
impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            default: default_model(),
            routing: None,
            claude: Some(ClaudeModelConfig {
                default_model: default_claude_model(),
            }),
            codex: Some(CodexModelConfig {
                default_model: default_codex_model(),
            }),
            ollama: None,
        }
    }
}

impl ModelsConfig {
    pub fn normalize_provider_name(target: &str) -> Option<&'static str> {
        let target = target.trim();
        if target.is_empty() {
            return None;
        }
        if target == "claude" || target.starts_with("claude-") {
            return Some("claude");
        }
        if target == "codex"
            || target.starts_with("codex")
            || target.starts_with("gpt-")
            || target.starts_with("o1")
            || target.starts_with("o3")
            || target.starts_with("o4")
            || target.starts_with("o5")
        {
            return Some("codex");
        }
        if target == "ollama" || target.starts_with("ollama/") {
            return Some("ollama");
        }
        if target == "mock" {
            return Some("mock");
        }
        None
    }

    pub fn default_provider(&self) -> String {
        Self::normalize_provider_name(&self.default)
            .unwrap_or("claude")
            .to_string()
    }

    pub fn default_model_for_provider(&self, provider: &str) -> String {
        match provider {
            "claude" => self
                .claude
                .as_ref()
                .map(|c| c.default_model.clone())
                .unwrap_or_else(default_claude_model),
            "codex" => self
                .codex
                .as_ref()
                .map(|c| c.default_model.clone())
                .unwrap_or_else(default_codex_model),
            "ollama" => self
                .ollama
                .as_ref()
                .map(|c| c.default_model.clone())
                .unwrap_or_else(default_ollama_model),
            "mock" => "mock".to_string(),
            _ => self.default_model_for_provider("claude"),
        }
    }

    pub fn resolve_provider_and_model(&self, requested: Option<&str>) -> (String, String) {
        let target = requested.unwrap_or(self.default.as_str()).trim();
        if target.is_empty() || target == "default" {
            let provider = self.default_provider();
            let model = self.default_model_for_provider(&provider);
            return (provider, model);
        }

        if let Some(model) = target.strip_prefix("ollama/") {
            return ("ollama".to_string(), model.to_string());
        }

        if let Some(provider) = Self::normalize_provider_name(target) {
            let model = if target == provider {
                self.default_model_for_provider(provider)
            } else {
                target.to_string()
            };
            return (provider.to_string(), model);
        }

        if self.ollama.is_some() {
            return ("ollama".to_string(), target.to_string());
        }

        let provider = self.default_provider();
        (provider, target.to_string())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RagConfig {
    pub enabled: bool,
    pub vector_db: String,
    pub embedding_provider: String,
    pub embedding_model: String,
    #[serde(default)]
    pub index: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryConfig {
    pub rag: Option<RagConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerCommandConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum McpServerEntry {
    Command(String),
    Config(McpServerCommandConfig),
}

impl McpServerEntry {
    pub fn to_server_config(&self, name: &str) -> agent007_mcp::McpServerConfig {
        match self {
            Self::Command(command) => agent007_mcp::McpServerConfig {
                name: name.to_string(),
                command: command.clone(),
                args: Vec::new(),
                env: HashMap::new(),
                cwd: None,
            },
            Self::Config(config) => agent007_mcp::McpServerConfig {
                name: name.to_string(),
                command: config.command.clone(),
                args: config.args.clone(),
                env: config.env.clone(),
                cwd: config.cwd.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, McpServerEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdeConfig {
    #[serde(default = "default_ide_port")]
    pub port: u16,
}
fn default_ide_port() -> u16 {
    7007
}
impl Default for IdeConfig {
    fn default() -> Self {
        Self {
            port: default_ide_port(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RewardWeightsConfig {
    #[serde(default = "default_completion_weight")]
    pub completion: f32,
    #[serde(default = "default_user_rating_weight")]
    pub user_rating: f32,
    #[serde(default = "default_tool_errors_weight")]
    pub tool_errors: f32,
    #[serde(default = "default_retries_weight")]
    pub retries: f32,
}
fn default_completion_weight() -> f32 {
    0.4
}
fn default_user_rating_weight() -> f32 {
    0.3
}
fn default_tool_errors_weight() -> f32 {
    0.2
}
fn default_retries_weight() -> f32 {
    0.1
}
impl Default for RewardWeightsConfig {
    fn default() -> Self {
        Self {
            completion: 0.4,
            user_rating: 0.3,
            tool_errors: 0.2,
            retries: 0.1,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LearningConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_optimizer_threshold")]
    pub optimizer_threshold: f32,
    #[serde(default = "default_optimizer_trigger_count")]
    pub optimizer_trigger_count: usize,
    #[serde(default = "default_optimizer_model")]
    pub optimizer_model: String,
    #[serde(default)]
    pub reward_weights: RewardWeightsConfig,
}
fn default_optimizer_threshold() -> f32 {
    0.3
}
fn default_optimizer_trigger_count() -> usize {
    10
}
fn default_optimizer_model() -> String {
    "claude".to_string()
}
impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            optimizer_threshold: 0.3,
            optimizer_trigger_count: 10,
            optimizer_model: "claude".to_string(),
            reward_weights: RewardWeightsConfig::default(),
        }
    }
}

/// Zone access levels for file paths (globs).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ZonesConfig {
    #[serde(default)]
    pub forbidden: Vec<String>,
    #[serde(default)]
    pub readonly: Vec<String>,
    #[serde(default)]
    pub sensitive: Vec<String>,
    #[serde(default)]
    pub unrestricted: Vec<String>,
}

fn default_lsp_enabled() -> bool {
    true
}
fn default_lsp_inject_categories() -> Vec<String> {
    vec!["code_completion".to_string(), "reasoning".to_string()]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LspConfig {
    #[serde(default = "default_lsp_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub servers: HashMap<String, String>,
    #[serde(default = "default_lsp_inject_categories")]
    pub inject_for_categories: Vec<String>,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            servers: HashMap::new(),
            inject_for_categories: default_lsp_inject_categories(),
        }
    }
}

impl LspConfig {
    /// Auto-detect LSP servers available on PATH.
    pub fn detect() -> Self {
        let mut servers = HashMap::new();
        if which_on_path("rust-analyzer") {
            servers.insert("rust".to_string(), "rust-analyzer".to_string());
        }
        if which_on_path("typescript-language-server") {
            servers.insert(
                "typescript".to_string(),
                "typescript-language-server --stdio".to_string(),
            );
            servers.insert(
                "javascript".to_string(),
                "typescript-language-server --stdio".to_string(),
            );
        }
        if which_on_path("pyright") {
            servers.insert("python".to_string(), "pyright --stdio".to_string());
        }
        if which_on_path("gopls") {
            servers.insert("go".to_string(), "gopls".to_string());
        }
        Self {
            enabled: true,
            servers,
            inject_for_categories: default_lsp_inject_categories(),
        }
    }
}

fn which_on_path(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub core: CoreConfig,
    #[serde(default)]
    pub models: ModelsConfig,
    pub memory: Option<MemoryConfig>,
    pub mcp: Option<McpConfig>,
    #[serde(default)]
    pub ide: IdeConfig,
    #[serde(default)]
    pub learning: LearningConfig,
    #[serde(default)]
    pub zones: ZonesConfig,
    #[serde(default)]
    pub lsp: Option<LspConfig>,
}

impl Config {
    pub fn from_str(s: &str) -> Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// Load config with layered resolution:
    /// 1. AGENT007_CONFIG env var (explicit override)
    /// 2. Project-local .agent007/config.toml (walk up from CWD)
    /// 3. Global ~/.agent007/config.toml
    /// 4. Built-in defaults
    pub fn load() -> Result<Self> {
        if let Ok(p) = std::env::var("AGENT007_CONFIG") {
            let path = PathBuf::from(p);
            if path.exists() {
                let s = std::fs::read_to_string(&path)?;
                return Self::from_str(&s);
            }
        }

        if let Some(project) = Self::project_config_path() {
            if project.exists() {
                let s = std::fs::read_to_string(&project)?;
                return Self::from_str(&s);
            }
        }

        let global = Self::global_config_path();
        if global.exists() {
            let s = std::fs::read_to_string(&global)?;
            return Self::from_str(&s);
        }

        Ok(Self::default())
    }

    /// Walk up from CWD looking for `.agent007/config.toml`.
    fn project_config_path() -> Option<PathBuf> {
        let mut dir = std::env::current_dir().ok()?;
        loop {
            let candidate = dir.join(".agent007").join("config.toml");
            if candidate.exists() {
                return Some(candidate);
            }
            if !dir.pop() {
                return None;
            }
        }
    }

    fn global_config_path() -> PathBuf {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".agent007")
            .join("config.toml")
    }

    /// Path to simulation templates directory.
    pub fn simulation_templates_dir(&self) -> PathBuf {
        crate::commands::run::agent007_home().join("simulation-templates")
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            core: CoreConfig::default(),
            models: ModelsConfig::default(),
            memory: None,
            mcp: None,
            ide: IdeConfig::default(),
            learning: LearningConfig::default(),
            zones: ZonesConfig::default(),
            lsp: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const SAMPLE_CONFIG: &str = r#"
[core]
max_agents = 4
task_queue_capacity = 128

[models]
default = "claude"

[models.routing]
code_completion = "codex"
reasoning = "claude"
fast_local = "ollama"
sensitive = "ollama"

[models.ollama]
base_url = "http://localhost:11434"
default_model = "llama3"

[memory.rag]
enabled = true
vector_db = "lancedb"
embedding_provider = "ollama"
embedding_model = "nomic-embed-text"
index = ["./src", "./docs"]

[mcp.servers]
filesystem = "npx @modelcontextprotocol/server-filesystem"

[ide]
port = 7007

[learning]
enabled = true
optimizer_threshold = 0.3
optimizer_trigger_count = 10
optimizer_model = "claude"

[learning.reward_weights]
completion = 0.4
user_rating = 0.3
tool_errors = 0.2
retries = 0.1
"#;

    #[test]
    fn parse_full_config_toml() {
        let config = Config::from_str(SAMPLE_CONFIG).unwrap();
        assert_eq!(config.core.max_agents, 4);
        assert_eq!(config.core.task_queue_capacity, 128);
        assert_eq!(config.models.default, "claude");
        assert_eq!(
            config
                .models
                .routing
                .as_ref()
                .unwrap()
                .code_completion
                .as_deref(),
            Some("codex")
        );
        assert_eq!(
            config.models.ollama.as_ref().unwrap().base_url,
            "http://localhost:11434"
        );
        assert_eq!(
            config.models.default_model_for_provider("claude"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            config.models.default_model_for_provider("codex"),
            "gpt-5.3-codex"
        );
        assert_eq!(
            config
                .memory
                .as_ref()
                .unwrap()
                .rag
                .as_ref()
                .unwrap()
                .enabled,
            true
        );
        assert_eq!(
            config
                .memory
                .as_ref()
                .unwrap()
                .rag
                .as_ref()
                .unwrap()
                .vector_db,
            "lancedb"
        );
        assert_eq!(config.ide.port, 7007);
        assert_eq!(config.learning.enabled, true);
        assert_eq!(config.learning.reward_weights.completion, 0.4);
    }

    #[test]
    fn config_defaults_are_sensible() {
        let config = Config::from_str("[core]\nmax_agents = 1").unwrap();
        assert_eq!(config.models.default, "claude");
        assert_eq!(config.ide.port, 7007);
        assert_eq!(config.learning.enabled, false);
    }

    #[test]
    fn models_config_resolves_provider_and_model() {
        let config = Config::default();
        assert_eq!(
            config.models.resolve_provider_and_model(Some("codex")),
            ("codex".to_string(), "gpt-5.3-codex".to_string())
        );
        assert_eq!(
            config
                .models
                .resolve_provider_and_model(Some("claude-sonnet-4-6")),
            ("claude".to_string(), "claude-sonnet-4-6".to_string())
        );
        assert_eq!(
            config.models.resolve_provider_and_model(Some("gpt-5.4")),
            ("codex".to_string(), "gpt-5.4".to_string())
        );
        assert_eq!(
            config
                .models
                .resolve_provider_and_model(Some("ollama/phi4")),
            ("ollama".to_string(), "phi4".to_string())
        );
    }

    #[test]
    fn structured_mcp_server_entries_parse() {
        let config = Config::from_str(
            r#"
[mcp.servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
cwd = "/tmp"

[mcp.servers.filesystem.env]
NODE_ENV = "production"
"#,
        )
        .unwrap();

        let entry = config
            .mcp
            .as_ref()
            .unwrap()
            .servers
            .get("filesystem")
            .unwrap()
            .to_server_config("filesystem");

        assert_eq!(entry.command, "npx");
        assert_eq!(entry.args[0], "-y");
        assert_eq!(entry.cwd.as_deref(), Some("/tmp"));
        assert_eq!(
            entry.env.get("NODE_ENV").map(String::as_str),
            Some("production")
        );
    }

    #[test]
    fn config_load_respects_agent007_config_env() {
        use std::io::Write;
        use tempfile::NamedTempFile;
        let _guard = ENV_LOCK.lock().unwrap();
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(SAMPLE_CONFIG.as_bytes()).unwrap();
        std::env::set_var("AGENT007_CONFIG", f.path());
        let config = Config::load().unwrap();
        std::env::remove_var("AGENT007_CONFIG");
        assert_eq!(config.core.max_agents, 4);
    }
}
