use crate::config::{HookConfig, HookEvent};
use crate::error::HookError;

pub struct HookExecutor {
    config: HookConfig,
}

impl HookExecutor {
    pub fn new(config: HookConfig) -> Self {
        Self { config }
    }

    /// Fire the hook for the given event. If no command is configured, returns Ok(()) immediately.
    /// Spawns the shell command via std::process::Command, waits for exit, returns error on non-zero.
    pub fn fire(&self, event: &HookEvent) -> Result<(), HookError> {
        let command = match self.config.command_for(event) {
            Some(cmd) => cmd.to_string(),
            None => return Ok(()),
        };

        tracing::debug!(event = ?event, command = %command, "firing hook");

        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(&command);

        // Pass event-specific context as environment variables
        match event {
            HookEvent::OnMemoryWrite { key } => {
                cmd.env("HOOK_KEY", key);
            }
            HookEvent::PreToolCall { tool } | HookEvent::PostToolCall { tool } => {
                cmd.env("HOOK_TOOL", tool);
            }
            HookEvent::OnSkillExecute { skill } => {
                cmd.env("HOOK_SKILL", skill);
            }
            _ => {}
        }

        let status = cmd.spawn()
            .map_err(|e| {
                tracing::warn!(command = %command, error = %e, "failed to spawn hook command");
                HookError::SpawnFailed { command: command.clone(), source: e }
            })?
            .wait()
            .map_err(|e| {
                tracing::warn!(command = %command, error = %e, "failed to wait for hook command");
                HookError::WaitFailed { command: command.clone(), source: e }
            })?;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            tracing::warn!(command = %command, code = code, "hook command failed");
            return Err(HookError::CommandFailed { command, code });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HookConfig, HookEvent};

    fn make_executor(pre_agent_run: Option<&str>) -> HookExecutor {
        HookExecutor::new(HookConfig {
            pre_agent_run: pre_agent_run.map(str::to_string),
            ..Default::default()
        })
    }

    #[test]
    fn fire_pre_agent_run_echo_hello_returns_ok() {
        let executor = make_executor(Some("echo hello"));
        let result = executor.fire(&HookEvent::PreAgentRun);
        assert!(result.is_ok(), "expected Ok(()), got: {:?}", result);
    }

    #[test]
    fn fire_post_agent_run_no_command_is_noop() {
        let executor = HookExecutor::new(HookConfig::default());
        let result = executor.fire(&HookEvent::PostAgentRun);
        assert!(result.is_ok(), "expected Ok(()) for no-op, got: {:?}", result);
    }

    #[test]
    fn fire_post_agent_run_empty_string_is_noop() {
        let executor = HookExecutor::new(HookConfig {
            post_agent_run: Some("".to_string()),
            ..Default::default()
        });
        let result = executor.fire(&HookEvent::PostAgentRun);
        assert!(result.is_ok(), "expected Ok(()) for empty command, got: {:?}", result);
    }

    #[test]
    fn fire_pre_agent_run_exit_1_returns_command_failed() {
        let executor = make_executor(Some("exit 1"));
        let result = executor.fire(&HookEvent::PreAgentRun);
        match result {
            Err(HookError::CommandFailed { command, code }) => {
                assert_eq!(command, "exit 1");
                assert_eq!(code, 1);
            }
            other => panic!("expected HookError::CommandFailed, got: {:?}", other),
        }
    }

    #[test]
    fn fire_on_memory_write_passes_key_via_env() {
        // The command echoes $HOOK_KEY; if it can be referenced, the command succeeds
        let executor = HookExecutor::new(HookConfig {
            on_memory_write: Some("test \"$HOOK_KEY\" = \"testkey\"".to_string()),
            ..Default::default()
        });
        let result = executor.fire(&HookEvent::OnMemoryWrite { key: "testkey".to_string() });
        assert!(result.is_ok(), "expected Ok(()), got: {:?}", result);
    }
}
