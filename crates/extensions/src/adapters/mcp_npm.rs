use crate::adapter::{AdapterError, ExtensionAdapter, ExtensionSource};
use crate::bundle::{CompatGrade, ExtensionBundle, ExtensionManifest, ManifestMeta};
use async_trait::async_trait;

pub struct McpNpmAdapter;

#[async_trait]
impl ExtensionAdapter for McpNpmAdapter {
    fn name(&self) -> &str {
        "mcp-npm"
    }
    fn can_handle(&self, source: &ExtensionSource) -> bool {
        matches!(
            source,
            ExtensionSource::McpNpm { .. } | ExtensionSource::Npm { .. }
        )
    }
    async fn fetch(&self, source: &ExtensionSource) -> Result<ExtensionBundle, AdapterError> {
        let package = match source {
            ExtensionSource::McpNpm { package } => package.clone(),
            ExtensionSource::Npm { package, .. } => package.clone(),
            _ => return Err(AdapterError::Unsupported),
        };
        let slug = package.trim_start_matches('@').replace(['/', '@'], "-");
        let mcp_entry = serde_json::json!({
            "name": slug,
            "source_kind": "npm",
            "source_ref": package,
            "command": "npx",
            "args": ["-y", package],
            "env": {},
            "approved": false,
            "tools": [],
            "status": "disconnected",
            "scope": "project",
            "added_at": chrono::Utc::now().to_rfc3339(),
        });
        let mut bundle = ExtensionBundle::default();
        bundle.compat_grade = Some(CompatGrade::B);
        bundle.manifest = Some(ExtensionManifest {
            extension: ManifestMeta {
                name: slug,
                version: "latest".to_string(),
                description: Some(format!("MCP server from npm package: {}", package)),
                author: None,
                compat: None,
                min_version: None,
                license: None,
                requires: None,
                permissions: None,
            },
        });
        bundle.mcp_servers = vec![mcp_entry];
        Ok(bundle)
    }
}
