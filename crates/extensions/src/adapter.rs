use crate::bundle::ExtensionBundle;
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum ExtensionSource {
    Local(PathBuf),
    GitHub {
        owner: String,
        repo: String,
        ref_: Option<String>,
    },
    Npm {
        package: String,
        version: Option<String>,
    },
    McpNpm {
        package: String,
    },
    Url(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("fetch failed: {0}")]
    Fetch(String),
    #[error("parse failed: {0}")]
    Parse(String),
    #[error("unsupported source")]
    Unsupported,
}

// Re-export AdaptResult as just the bundle (warnings are inside)
pub type AdaptResult = ExtensionBundle;

#[async_trait]
pub trait ExtensionAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn can_handle(&self, source: &ExtensionSource) -> bool;
    async fn fetch(&self, source: &ExtensionSource) -> Result<ExtensionBundle, AdapterError>;
}
