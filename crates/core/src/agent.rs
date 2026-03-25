use crate::types::AgentId;

#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    Idle,
    Running,
    Done,
    Failed(String),
}

pub struct AgentHandle {
    pub id: AgentId,
    pub state: AgentState,
}

impl AgentHandle {
    pub fn new() -> Self {
        Self { id: AgentId::new(), state: AgentState::Idle }
    }
}

impl Default for AgentHandle {
    fn default() -> Self { Self::new() }
}
