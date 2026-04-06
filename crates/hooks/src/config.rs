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

/// On-disk entry in the `[[hooks]]` array format.
#[derive(Debug, Deserialize)]
struct HookEntry {
    event: String,
    command: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool { true }

/// On-disk wrapper for the `[[hooks]]` array format.
#[derive(Debug, Deserialize)]
struct ArrayHookFile {
    #[serde(default, rename = "hooks")]
    hooks: Vec<HookEntry>,
}

impl HookConfig {
    pub fn load(path: &Path) -> Result<Self, crate::error::HookError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::error::HookError::ConfigRead { path: path.to_path_buf(), source: e })?;

        // Try flat format first (`pre_agent_run = "cmd"`).
        // If that succeeds and at least one field is set, use it.
        if let Ok(flat) = toml::from_str::<HookConfig>(&content) {
            if flat.pre_agent_run.is_some()
                || flat.post_agent_run.is_some()
                || flat.pre_tool_call.is_some()
                || flat.post_tool_call.is_some()
                || flat.on_memory_write.is_some()
                || flat.on_skill_execute.is_some()
                || flat.post_task_complete.is_some()
            {
                return Ok(flat);
            }
        }

        // Fall back to `[[hooks]]` array format.
        let file: ArrayHookFile = toml::from_str(&content)
            .map_err(crate::error::HookError::ConfigParse)?;

        let mut cfg = HookConfig::default();
        for entry in file.hooks {
            if !entry.enabled || entry.command.is_empty() {
                continue;
            }
            let cmd = Some(entry.command);
            match entry.event.as_str() {
                "task_start" | "pre_agent_run"   => cfg.pre_agent_run   = cmd,
                "task_complete" | "post_agent_run" => cfg.post_agent_run = cmd,
                "post_task_complete"               => cfg.post_task_complete = cmd,
                "pre_tool_call"                    => cfg.pre_tool_call  = cmd,
                "post_tool_call" | "tool_blocked"  => cfg.post_tool_call = cmd,
                "on_memory_write"                  => cfg.on_memory_write = cmd,
                "on_skill_execute"                 => cfg.on_skill_execute = cmd,
                other => {
                    tracing::warn!(event = other, "unknown hook event name in hooks.toml — skipping");
                }
            }
        }
        Ok(cfg)
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
