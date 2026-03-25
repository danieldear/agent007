use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdeBridgeError {
    #[error("LSP transport error: {0}")]
    Transport(String),

    #[error("command not found: {0}")]
    UnknownCommand(String),

    #[error("missing required argument: {0}")]
    MissingArgument(String),

    #[error("orchestrator error: {0}")]
    Orchestrator(#[from] agent007_core::CoreError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
