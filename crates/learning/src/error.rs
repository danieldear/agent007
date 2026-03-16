use thiserror::Error;

#[derive(Debug, Error)]
pub enum LearningError {
    #[error("memory error: {0}")]
    Memory(#[from] agent007_memory::MemoryError),

    #[error("model error: {0}")]
    Model(#[from] agent007_models::ModelError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("feedback entry not found: {0}")]
    NotFound(uuid::Uuid),

    #[error("optimizer failed for skill '{skill}': {reason}")]
    OptimizerFailed { skill: String, reason: String },

    #[error("dispatcher error: {0}")]
    Dispatcher(String),
}
