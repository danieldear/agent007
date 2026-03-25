use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use anyhow::Result;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CoreConfig {
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
    #[serde(default = "default_task_queue_capacity")]
    pub task_queue_capacity: usize,
}
fn default_max_agents() -> usize { 8 }
fn default_task_queue_capacity() -> usize { 256 }
impl Default for CoreConfig {
    fn default() -> Self { Self { max_agents: default_max_agents(), task_queue_capacity: default_task_queue_capacity() } }
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
fn default_ollama_url() -> String { "http://localhost:11434".to_string() }
fn default_ollama_model() -> String { "llama3".to_string() }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelsConfig {
    #[serde(default = "default_model")]
    pub default: String,
    pub routing: Option<RoutingConfig>,
    pub ollama: Option<OllamaModelConfig>,
}
fn default_model() -> String { "claude".to_string() }
impl Default for ModelsConfig {
    fn default() -> Self { Self { default: default_model(), routing: None, ollama: None } }
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdeConfig {
    #[serde(default = "default_ide_port")]
    pub port: u16,
}
fn default_ide_port() -> u16 { 7007 }
impl Default for IdeConfig {
    fn default() -> Self { Self { port: default_ide_port() } }
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
fn default_completion_weight() -> f32 { 0.4 }
fn default_user_rating_weight() -> f32 { 0.3 }
fn default_tool_errors_weight() -> f32 { 0.2 }
fn default_retries_weight() -> f32 { 0.1 }
impl Default for RewardWeightsConfig {
    fn default() -> Self { Self { completion: 0.4, user_rating: 0.3, tool_errors: 0.2, retries: 0.1 } }
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
fn default_optimizer_threshold() -> f32 { 0.3 }
fn default_optimizer_trigger_count() -> usize { 10 }
fn default_optimizer_model() -> String { "claude".to_string() }
impl Default for LearningConfig {
    fn default() -> Self { Self { enabled: false, optimizer_threshold: 0.3, optimizer_trigger_count: 10, optimizer_model: "claude".to_string(), reward_weights: RewardWeightsConfig::default() } }
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
}

impl Config {
    pub fn from_str(s: &str) -> Result<Self> {
        Ok(toml::from_str(s)?)
    }

    pub fn load() -> Result<Self> {
        let path = if let Ok(p) = std::env::var("AGENT007_CONFIG") {
            PathBuf::from(p)
        } else {
            Self::default_path()
        };
        if path.exists() {
            let s = std::fs::read_to_string(&path)?;
            Self::from_str(&s)
        } else {
            Ok(Self::default())
        }
    }

    fn default_path() -> PathBuf {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".agent007")
            .join("config.toml")
    }

    /// Path to `~/.agent007/`
    pub fn agent007_home(&self) -> PathBuf {
        std::env::var("AGENT007_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(".agent007")
            })
    }

    /// Path to simulation templates directory (built-in + user).
    pub fn simulation_templates_dir(&self) -> PathBuf {
        self.agent007_home().join("simulation-templates")
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
        assert_eq!(config.models.routing.as_ref().unwrap().code_completion.as_deref(), Some("codex"));
        assert_eq!(config.models.ollama.as_ref().unwrap().base_url, "http://localhost:11434");
        assert_eq!(config.memory.as_ref().unwrap().rag.as_ref().unwrap().enabled, true);
        assert_eq!(config.memory.as_ref().unwrap().rag.as_ref().unwrap().vector_db, "lancedb");
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
    fn config_load_respects_agent007_config_env() {
        use tempfile::NamedTempFile;
        use std::io::Write;
        let _guard = ENV_LOCK.lock().unwrap();
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(SAMPLE_CONFIG.as_bytes()).unwrap();
        std::env::set_var("AGENT007_CONFIG", f.path());
        let config = Config::load().unwrap();
        std::env::remove_var("AGENT007_CONFIG");
        assert_eq!(config.core.max_agents, 4);
    }
}
