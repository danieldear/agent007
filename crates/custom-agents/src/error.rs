use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum CustomAgentError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse agent file {path}: {reason}")]
    ParseError { path: PathBuf, reason: String },
    #[error("agent '{name}' not found")]
    NotFound { name: String },
    #[error("max orchestrator depth {max} exceeded")]
    MaxDepthExceeded { max: usize },
    #[error("worker '{name}' not in allowed_workers for this sub-orchestrator")]
    WorkerNotAllowed { name: String },
    #[error("worker persona '{name}' not found")]
    WorkerPersonaNotFound { name: String },
    #[error("persona '{name}' must have agent_type = '{expected}' for this use")]
    InvalidPersonaType { name: String, expected: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn not_found_message() {
        let e = CustomAgentError::NotFound { name: "Foo".into() };
        assert_eq!(e.to_string(), "agent 'Foo' not found");
    }

    #[test]
    fn parse_error_message() {
        let e = CustomAgentError::ParseError {
            path: PathBuf::from("agents/foo.toml"),
            reason: "missing field `name`".into(),
        };
        assert!(e.to_string().contains("agents/foo.toml"));
        assert!(e.to_string().contains("missing field `name`"));
    }

    #[test]
    fn max_depth_message() {
        let e = CustomAgentError::MaxDepthExceeded { max: 3 };
        assert!(e.to_string().contains('3'));
    }

    #[test]
    fn worker_not_allowed_message() {
        let e = CustomAgentError::WorkerNotAllowed {
            name: "Hacker".into(),
        };
        assert!(e.to_string().contains("Hacker"));
    }

    #[test]
    fn worker_persona_not_found_message() {
        let e = CustomAgentError::WorkerPersonaNotFound {
            name: "Missing".into(),
        };
        assert!(e.to_string().contains("Missing"));
    }

    #[test]
    fn invalid_persona_type_message() {
        let e = CustomAgentError::InvalidPersonaType {
            name: "Coder".into(),
            expected: "orchestrator".into(),
        };
        assert!(e.to_string().contains("Coder"));
        assert!(e.to_string().contains("orchestrator"));
    }
}
