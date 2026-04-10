use agent007_memory::MemoryError;
use agent007_models::ModelError;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Missing frontmatter in skill file: {path}")]
    MissingFrontmatter { path: PathBuf },

    #[error("Frontmatter parse error in {path}: {source}")]
    FrontmatterParse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("Template render error in skill '{name}': {source}")]
    TemplateRender {
        name: String,
        #[source]
        source: tera::Error,
    },

    #[error("Model error in skill '{name}': {source}")]
    Model { name: String, source: ModelError },

    #[error("Memory error in skill '{name}': {source}")]
    Memory { name: String, source: MemoryError },
}
