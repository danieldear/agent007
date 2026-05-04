use crate::adapter::{AdapterError, ExtensionAdapter, ExtensionSource};
use crate::bundle::{BundleFile, CompatGrade, ExtensionBundle, ExtensionManifest, ManifestMeta};
use async_trait::async_trait;

pub struct ClaudeMarketplaceAdapter {
    client: reqwest::Client,
}

impl ClaudeMarketplaceAdapter {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("agent007-extensions/0.1")
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap(),
        }
    }

    /// Resolve a URL or GitHub repo URL to the raw marketplace.json URL.
    fn resolve_url(url: &str) -> String {
        // Already a direct marketplace.json or .claude-plugin path
        if url.contains("marketplace.json") || url.contains(".claude-plugin") {
            return url.to_string();
        }
        // GitHub repo URL — try raw main branch .claude-plugin/marketplace.json
        if let Some(path) = url.strip_prefix("https://github.com/") {
            let parts: Vec<&str> = path.trim_end_matches('/').splitn(2, '/').collect();
            if parts.len() == 2 {
                let owner = parts[0];
                let repo = parts[1];
                return format!(
                    "https://raw.githubusercontent.com/{}/{}/main/.claude-plugin/marketplace.json",
                    owner, repo
                );
            }
        }
        if let Some(path) = url.strip_prefix("github.com/") {
            let parts: Vec<&str> = path.trim_end_matches('/').splitn(2, '/').collect();
            if parts.len() == 2 {
                let owner = parts[0];
                let repo = parts[1];
                return format!(
                    "https://raw.githubusercontent.com/{}/{}/main/.claude-plugin/marketplace.json",
                    owner, repo
                );
            }
        }
        // Unknown URL — try as-is
        url.to_string()
    }

    /// Build a skill `.md` file with YAML frontmatter from a marketplace skill entry.
    fn skill_to_md(skill: &serde_json::Value, pkg_name: &str) -> BundleFile {
        let name = skill["name"].as_str().unwrap_or("skill").to_string();
        let description = skill["description"].as_str().unwrap_or("").to_string();
        let trigger = skill["trigger"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| format!("/{}", name.to_lowercase().replace(' ', "-")));
        let template = skill["template"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| format!(
                "You are a helpful assistant. The user has invoked the `{}` skill.\n\nComplete the user's request thoroughly and clearly.",
                name
            ));

        let content = format!(
            "---\nname: {name}\ntrigger: {trigger}\ndescription: {description}\nmodel: claude-opus-4-7\n---\n{template}\n",
        );

        let file_name = format!("{}-{}.md", pkg_name, name.to_lowercase().replace(' ', "-"));
        BundleFile {
            name: file_name,
            content,
        }
    }
}

impl Default for ClaudeMarketplaceAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExtensionAdapter for ClaudeMarketplaceAdapter {
    fn name(&self) -> &str {
        "claude-marketplace"
    }

    fn can_handle(&self, source: &ExtensionSource) -> bool {
        match source {
            ExtensionSource::Url(url) => {
                url.contains("claude-plugin") || url.contains("marketplace.json")
            }
            // Fallback for GitHub sources not handled upstream (won't normally reach here since
            // GitHubAdapter is first, but kept for completeness)
            ExtensionSource::GitHub { .. } => false,
            _ => false,
        }
    }

    async fn fetch(&self, source: &ExtensionSource) -> Result<ExtensionBundle, AdapterError> {
        let raw_url = match source {
            ExtensionSource::Url(url) => Self::resolve_url(url),
            ExtensionSource::GitHub { owner, repo, ref_ } => {
                let branch = ref_.as_deref().unwrap_or("main");
                format!(
                    "https://raw.githubusercontent.com/{}/{}/{}/.claude-plugin/marketplace.json",
                    owner, repo, branch
                )
            }
            _ => return Err(AdapterError::Unsupported),
        };

        let resp = self
            .client
            .get(&raw_url)
            .send()
            .await
            .map_err(|e| AdapterError::Fetch(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(AdapterError::Fetch(format!(
                "HTTP {} fetching marketplace.json from {}",
                resp.status(),
                raw_url
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AdapterError::Parse(format!("invalid JSON: {}", e)))?;

        // Lenient parsing — missing fields get sensible defaults
        let name = json["name"]
            .as_str()
            .unwrap_or("marketplace-extension")
            .to_string();
        let version = json["version"].as_str().unwrap_or("0.0.0").to_string();
        let description = json["description"].as_str().map(String::from);

        let pkg_slug = name.to_lowercase().replace(' ', "-");

        let mut skills = vec![];
        if let Some(skill_arr) = json["skills"].as_array() {
            for skill in skill_arr {
                skills.push(Self::skill_to_md(skill, &pkg_slug));
            }
        }

        let mut bundle = ExtensionBundle::default();
        bundle.compat_grade = Some(CompatGrade::B);
        bundle.manifest = Some(ExtensionManifest {
            extension: ManifestMeta {
                name: pkg_slug,
                version,
                description,
                author: None,
                compat: None,
                min_version: None,
                license: None,
                requires: None,
                permissions: None,
            },
        });
        bundle.skills = skills;
        bundle.warnings.push(
            "Imported from Claude marketplace — skill templates adapted, review before use"
                .to_string(),
        );
        Ok(bundle)
    }
}
