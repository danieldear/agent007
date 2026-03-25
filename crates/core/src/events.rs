use serde::{Deserialize, Serialize};

use crate::task::{Task, TaskResult};
use crate::types::{AgentId, MemoryRef, PromptRef};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookEvent {
    PreAgentRun,
    PostAgentRun,
    PreToolCall,
    PostToolCall,
    OnMemoryWrite,
    OnSkillExecute,
    PostTaskComplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    TaskAssigned { agent_id: AgentId, task: Task },
    TaskCompleted { agent_id: AgentId, result: TaskResult },
    ToolCall { agent_id: AgentId, tool: ToolCall },
    MemoryWrite { key: String, value_ref: MemoryRef },
    HookFired { event: HookEvent },
    ModelRequest { provider: String, prompt_ref: PromptRef, token_estimate: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PromptRef;

    #[test]
    fn agent_event_clones_cleanly() {
        let e = AgentEvent::ModelRequest {
            provider: "claude".to_string(),
            prompt_ref: PromptRef::new(),
            token_estimate: 100,
        };
        let c = e.clone();
        assert!(matches!(c, AgentEvent::ModelRequest { token_estimate: 100, .. }));
    }

    #[test]
    fn memory_write_uses_opaque_ref() {
        let e = AgentEvent::MemoryWrite { key: "user.md".to_string(), value_ref: crate::types::MemoryRef::new() };
        if let AgentEvent::MemoryWrite { key, .. } = e {
            assert_eq!(key, "user.md");
        }
    }
}
