use crate::adapter::{AdapterError, ExtensionAdapter, ExtensionSource};
use crate::bundle::{BundleFile, CompatGrade, ExtensionBundle, ExtensionManifest, ManifestMeta};
use async_trait::async_trait;
use std::collections::HashSet;

pub struct GitHubAdapter {
    client: reqwest::Client,
}

impl GitHubAdapter {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("agent007-extensions/0.1")
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap(),
        }
    }

    async fn fetch_default_branch(&self, owner: &str, repo: &str) -> Option<String> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}");
        let resp = self.client.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body = resp.json::<serde_json::Value>().await.ok()?;
        body.get("default_branch")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    async fn fetch_text_if_success(&self, url: &str) -> Result<Option<String>, AdapterError> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| AdapterError::Fetch(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(AdapterError::Fetch(format!(
                "HTTP {} for {url}",
                resp.status()
            )));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| AdapterError::Fetch(e.to_string()))?;
        Ok(Some(text))
    }

    async fn fetch_tree_paths(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Vec<String>, AdapterError> {
        let url =
            format!("https://api.github.com/repos/{owner}/{repo}/git/trees/{branch}?recursive=1");
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| AdapterError::Fetch(e.to_string()))?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let body = resp
            .json::<serde_json::Value>()
            .await
            .map_err(|e| AdapterError::Parse(e.to_string()))?;
        let mut paths = Vec::new();
        if let Some(tree) = body.get("tree").and_then(|v| v.as_array()) {
            for item in tree {
                if item.get("type").and_then(|v| v.as_str()) != Some("blob") {
                    continue;
                }
                let Some(path) = item.get("path").and_then(|v| v.as_str()) else {
                    continue;
                };
                paths.push(path.to_string());
            }
        }
        Ok(paths)
    }

    async fn collect_bundle_files(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        tree_paths: &[String],
        section: &str,
    ) -> Result<Vec<BundleFile>, AdapterError> {
        let prefix = format!("{section}/");
        let raw_base = format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}");
        let mut files = Vec::new();
        for path in tree_paths {
            if !path.starts_with(&prefix) {
                continue;
            }
            let rel = path.trim_start_matches(&prefix);
            if rel.is_empty() {
                continue;
            }
            let url = format!("{raw_base}/{path}");
            let Some(content) = self.fetch_text_if_success(&url).await? else {
                continue;
            };
            files.push(BundleFile {
                name: rel.to_string(),
                content,
            });
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(files)
    }
}

impl Default for GitHubAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExtensionAdapter for GitHubAdapter {
    fn name(&self) -> &str {
        "github"
    }
    fn can_handle(&self, source: &ExtensionSource) -> bool {
        matches!(source, ExtensionSource::GitHub { .. })
    }
    async fn fetch(&self, source: &ExtensionSource) -> Result<ExtensionBundle, AdapterError> {
        let (owner, repo, ref_) = match source {
            ExtensionSource::GitHub { owner, repo, ref_ } => (owner, repo, ref_),
            _ => return Err(AdapterError::Unsupported),
        };
        let mut branches = Vec::new();
        if let Some(explicit) = ref_.clone() {
            branches.push(explicit);
        } else {
            if let Some(default_branch) = self.fetch_default_branch(owner, repo).await {
                branches.push(default_branch);
            }
            branches.push("main".to_string());
            branches.push("master".to_string());
        }
        let mut uniq = HashSet::new();
        branches.retain(|b| uniq.insert(b.clone()));

        for branch in branches {
            let raw_base = format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}");
            let manifest = match self
                .fetch_text_if_success(&format!("{raw_base}/manifest.toml"))
                .await?
            {
                Some(text) => toml::from_str::<ExtensionManifest>(&text).ok(),
                None => None,
            };
            let agent007_json = match self
                .fetch_text_if_success(&format!("{raw_base}/agent007.json"))
                .await?
            {
                Some(text) => serde_json::from_str::<serde_json::Value>(&text).ok(),
                None => None,
            };
            let tree_paths = self.fetch_tree_paths(owner, repo, &branch).await?;
            let skills = self
                .collect_bundle_files(owner, repo, &branch, &tree_paths, "skills")
                .await?;
            let tools = self
                .collect_bundle_files(owner, repo, &branch, &tree_paths, "tools")
                .await?;
            let workflows = self
                .collect_bundle_files(owner, repo, &branch, &tree_paths, "workflows")
                .await?;

            if manifest.is_none()
                && agent007_json.is_none()
                && skills.is_empty()
                && tools.is_empty()
                && workflows.is_empty()
            {
                continue;
            }

            let mut bundle = ExtensionBundle::default();
            bundle.compat_grade = Some(if manifest.is_some() || agent007_json.is_some() {
                CompatGrade::A
            } else {
                CompatGrade::C
            });
            bundle.manifest = if let Some(manifest) = manifest {
                Some(manifest)
            } else if let Some(json) = agent007_json {
                let name = json
                    .pointer("/agent007/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(repo)
                    .to_string();
                let version = json
                    .pointer("/agent007/version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0.0.0")
                    .to_string();
                Some(ExtensionManifest {
                    extension: ManifestMeta {
                        name,
                        version,
                        description: None,
                        author: Some(owner.clone()),
                        compat: None,
                        min_version: None,
                        license: None,
                        requires: None,
                        permissions: None,
                    },
                })
            } else {
                Some(ExtensionManifest {
                    extension: ManifestMeta {
                        name: repo.clone(),
                        version: "unknown".to_string(),
                        description: Some(format!("github.com/{owner}/{repo}")),
                        author: Some(owner.clone()),
                        compat: None,
                        min_version: None,
                        license: None,
                        requires: None,
                        permissions: None,
                    },
                })
            };
            bundle.skills = skills;
            bundle.tools = tools;
            bundle.workflows = workflows;
            if bundle.compat_grade == Some(CompatGrade::C) {
                bundle.warnings.push(
                    "No manifest.toml or agent007.json found — imported discovered files as Grade C. Add manifest for full compatibility.".to_string(),
                );
            }
            return Ok(bundle);
        }

        // Grade C fallback — metadata only
        let mut bundle = ExtensionBundle::default();
        bundle.compat_grade = Some(CompatGrade::C);
        bundle.manifest = Some(ExtensionManifest {
            extension: ManifestMeta {
                name: repo.clone(),
                version: "unknown".to_string(),
                description: Some(format!("github.com/{owner}/{repo}")),
                author: Some(owner.clone()),
                compat: None,
                min_version: None,
                license: None,
                requires: None,
                permissions: None,
            },
        });
        bundle.warnings.push(
            "No manifest.toml or agent007.json found — Grade C metadata only. Add these files to the repo for full import.".to_string(),
        );
        Ok(bundle)
    }
}
