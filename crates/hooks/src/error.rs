use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HookError {
    #[error("failed to read hooks config at {path}: {source}")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse hooks.toml: {0}")]
    ConfigParse(#[from] toml::de::Error),

    #[error("hook command failed with exit code {code}: {command}")]
    CommandFailed { command: String, code: i32 },

    #[error("failed to spawn hook command '{command}': {source}")]
    SpawnFailed {
        command: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to wait for hook command '{command}': {source}")]
    WaitFailed {
        command: String,
        #[source]
        source: std::io::Error,
    },
}
