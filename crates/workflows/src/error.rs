use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum WorkflowError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse workflow {path}: {reason}")]
    ParseError { path: PathBuf, reason: String },

    #[error("step '{id}' references unknown input '{input}'")]
    UnknownInput { id: String, input: String },

    #[error("workflow schema invalid: {reason}")]
    SchemaError { reason: String },

    #[error("workflow has a dependency cycle")]
    CycleDetected,

    #[error("step '{id}' failed: {reason}")]
    StepFailed { id: String, reason: String },

    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("approval denied for step '{0}'")]
    ApprovalDenied(String),

    #[error("approval required for step '{id}'")]
    ApprovalRequired { id: String },

    #[error("template render error for step '{id}': {reason}")]
    TemplateError { id: String, reason: String },

    #[error("persona '{0}' not found")]
    PersonaNotFound(String),

    #[error("evaluator '{id}' exceeded max retries ({max})")]
    MaxRetriesExceeded { id: String, max: u32 },

    #[error("router '{id}' found no matching route for output '{output}'")]
    NoRouteMatch { id: String, output: String },

    #[error("evaluator step '{id}' is invalid: {reason}")]
    InvalidEvaluator { id: String, reason: String },

    #[error("router step '{id}' is invalid: {reason}")]
    InvalidRouter { id: String, reason: String },

    #[error("skill '{0}' not found in .agent007/skills/")]
    SkillNotFound(String),

    #[error("eval gate blocked workflow '{workflow}': {reason}")]
    EvalGateBlocked { workflow: String, reason: String },
}
