use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum WorkflowError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse workflow {path}: {reason}")]
    ParseError { path: PathBuf, reason: String },

    #[error("step '{id}' references unknown input '{input}'")]
    UnknownInput { id: String, input: String },

    #[error("workflow has a dependency cycle")]
    CycleDetected,

    #[error("step '{id}' failed: {reason}")]
    StepFailed { id: String, reason: String },

    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("approval denied for step '{0}'")]
    ApprovalDenied(String),

    #[error("template render error for step '{id}': {reason}")]
    TemplateError { id: String, reason: String },

    #[error("persona '{0}' not found")]
    PersonaNotFound(String),
}
