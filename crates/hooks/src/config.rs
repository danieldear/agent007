use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct HookConfig {
    pub pre_agent_run: Option<String>,
    pub post_agent_run: Option<String>,
    pub pre_tool_call: Option<String>,
    pub post_tool_call: Option<String>,
    pub on_memory_write: Option<String>,
    pub on_skill_execute: Option<String>,
    pub post_task_complete: Option<String>,
}

impl HookConfig {
    pub fn load(path: &Path) -> Result<Self, crate::error::HookError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::error::HookError::ConfigRead { path: path.to_path_buf(), source: e })?;
        toml::from_str(&content).map_err(crate::error::HookError::ConfigParse)
    }

    pub fn command_for(&self, event: &HookEvent) -> Option<&str> {
        let cmd = match event {
            HookEvent::PreAgentRun => self.pre_agent_run.as_deref(),
            HookEvent::PostAgentRun => self.post_agent_run.as_deref(),
            HookEvent::PreToolCall { .. } => self.pre_tool_call.as_deref(),
            HookEvent::PostToolCall { .. } => self.post_tool_call.as_deref(),
            HookEvent::OnMemoryWrite { .. } => self.on_memory_write.as_deref(),
            HookEvent::OnSkillExecute { .. } => self.on_skill_execute.as_deref(),
            HookEvent::PostTaskComplete => self.post_task_complete.as_deref(),
        };
        cmd.filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone)]
pub enum HookEvent {
    PreAgentRun,
    PostAgentRun,
    PreToolCall { tool: String },
    PostToolCall { tool: String },
    OnMemoryWrite { key: String },
    OnSkillExecute { skill: String },
    PostTaskComplete,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn parse_hooks_toml_with_values() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"
pre_agent_run = "echo pre"
post_agent_run = "echo post"
"#).unwrap();
        let cfg = HookConfig::load(f.path()).unwrap();
        assert_eq!(cfg.pre_agent_run.as_deref(), Some("echo pre"));
        assert_eq!(cfg.post_agent_run.as_deref(), Some("echo post"));
        assert_eq!(cfg.on_memory_write, None);
    }

    #[test]
    fn empty_string_command_returns_none() {
        let cfg = HookConfig {
            pre_agent_run: Some("".to_string()),
            ..Default::default()
        };
        assert_eq!(cfg.command_for(&HookEvent::PreAgentRun), None);
    }

    #[test]
    fn missing_file_returns_error() {
        let result = HookConfig::load(std::path::Path::new("/nonexistent/hooks.toml"));
        assert!(result.is_err());
    }
}
