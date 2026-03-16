use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Task queue full — backpressure limit reached")]
    TaskQueueFull,

    #[error("Dispatcher publish failed: {0}")]
    DispatchFailed(String),

    #[error("Model error: {0}")]
    Model(#[from] agent007_models::ModelError),

    #[error("Shutdown in progress")]
    ShuttingDown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_displays_agent_id() {
        let e = CoreError::AgentNotFound("abc-123".to_string());
        assert!(e.to_string().contains("abc-123"));
    }
}

