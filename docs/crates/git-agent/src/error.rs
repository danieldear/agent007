// crates/git-agent/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum GitAgentError {
    #[error("git2 error: {0}")]
    Git2(#[from] git2::Error),
    #[error("no git repository found at {path}")]
    NoRepo { path: std::path::PathBuf },
    #[error("checkpoint not found: {name}")]
    CheckpointNotFound { name: String },
    #[error("impact analysis failed: {0}")]
    ImpactAnalysis(String),
    #[error("GitHub/GitLab API error: {0}")]
    ApiError(String),
    #[error("missing auth token (set GITHUB_TOKEN or GITLAB_TOKEN)")]
    MissingToken,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_repo_error_displays_path() {
        let err = GitAgentError::NoRepo {
            path: std::path::PathBuf::from("/tmp/missing"),
        };
        assert!(err.to_string().contains("/tmp/missing"));
    }

    #[test]
    fn checkpoint_not_found_displays_name() {
        let err = GitAgentError::CheckpointNotFound {
            name: "before-refactor".to_string(),
        };
        assert!(err.to_string().contains("before-refactor"));
    }

    #[test]
    fn missing_token_has_descriptive_message() {
        let err = GitAgentError::MissingToken;
        assert!(err.to_string().contains("GITHUB_TOKEN"));
    }
}
