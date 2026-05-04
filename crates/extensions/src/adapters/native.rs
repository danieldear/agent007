use crate::adapter::{AdapterError, ExtensionAdapter, ExtensionSource};
use crate::bundle::{BundleFile, CompatGrade, ExtensionBundle};
use async_trait::async_trait;
use std::path::Path;

pub struct NativeAdapter;

#[async_trait]
impl ExtensionAdapter for NativeAdapter {
    fn name(&self) -> &str {
        "native"
    }
    fn can_handle(&self, source: &ExtensionSource) -> bool {
        if let ExtensionSource::Local(path) = source {
            path.join("manifest.toml").exists()
        } else {
            false
        }
    }
    async fn fetch(&self, source: &ExtensionSource) -> Result<ExtensionBundle, AdapterError> {
        let path = match source {
            ExtensionSource::Local(p) => p,
            _ => return Err(AdapterError::Unsupported),
        };
        let manifest_str = tokio::fs::read_to_string(path.join("manifest.toml"))
            .await
            .map_err(|e| AdapterError::Fetch(e.to_string()))?;
        let manifest: crate::bundle::ExtensionManifest =
            toml::from_str(&manifest_str).map_err(|e| AdapterError::Parse(e.to_string()))?;

        let mut bundle = ExtensionBundle::default();
        bundle.compat_grade = Some(CompatGrade::A);
        bundle.manifest = Some(manifest);

        // scan sections recursively and preserve relative file paths
        bundle.skills = read_dir_files_recursive(&path.join("skills")).await;
        bundle.tools = read_dir_files_recursive(&path.join("tools")).await;
        bundle.workflows = read_dir_files_recursive(&path.join("workflows")).await;

        Ok(bundle)
    }
}

async fn read_dir_files_recursive(root: &Path) -> Vec<BundleFile> {
    let mut files = vec![];
    if !root.exists() {
        return files;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.is_file() {
                let Ok(content) = tokio::fs::read_to_string(&path).await else {
                    continue;
                };
                let Ok(rel) = path.strip_prefix(root) else {
                    continue;
                };
                let name = rel.to_string_lossy().replace('\\', "/");
                if !name.is_empty() {
                    files.push(BundleFile { name, content });
                }
            }
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}
