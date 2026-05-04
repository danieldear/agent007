use crate::adapter::{AdapterError, ExtensionAdapter, ExtensionSource};
use crate::bundle::{BundleFile, CompatGrade, ExtensionBundle, ExtensionManifest, ManifestMeta};
use async_trait::async_trait;

pub struct OpenApiAdapter {
    client: reqwest::Client,
}

impl OpenApiAdapter {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("agent007-extensions/0.1")
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap(),
        }
    }
}

impl Default for OpenApiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_spec_text(raw: &str) -> Result<serde_json::Value, AdapterError> {
    serde_json::from_str::<serde_json::Value>(raw).or_else(|json_err| {
        serde_yaml::from_str::<serde_json::Value>(raw).map_err(|yaml_err| {
            AdapterError::Parse(format!(
                "unable to parse OpenAPI as JSON ({json_err}) or YAML ({yaml_err})"
            ))
        })
    })
}

#[async_trait]
impl ExtensionAdapter for OpenApiAdapter {
    fn name(&self) -> &str {
        "openapi"
    }
    fn can_handle(&self, source: &ExtensionSource) -> bool {
        if let ExtensionSource::Url(url) = source {
            let lower = url.to_ascii_lowercase();
            (lower.contains("openapi")
                || lower.ends_with(".json")
                || lower.ends_with(".yaml")
                || lower.ends_with(".yml"))
                && !lower.contains("marketplace.json")
        } else {
            false
        }
    }
    async fn fetch(&self, source: &ExtensionSource) -> Result<ExtensionBundle, AdapterError> {
        let url = match source {
            ExtensionSource::Url(u) => u,
            _ => return Err(AdapterError::Unsupported),
        };
        let raw = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| AdapterError::Fetch(e.to_string()))?
            .text()
            .await
            .map_err(|e| AdapterError::Fetch(e.to_string()))?;
        let spec = parse_spec_text(&raw)?;

        let title = spec
            .pointer("/info/title")
            .and_then(|v| v.as_str())
            .unwrap_or("api")
            .to_string();
        let version = spec
            .pointer("/info/version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0")
            .to_string();
        let base_url = spec
            .pointer("/servers/0/url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut tools = vec![];
        if let Some(paths) = spec["paths"].as_object() {
            for (path, methods_val) in paths {
                if let Some(methods) = methods_val.as_object() {
                    for (method, op) in methods {
                        if matches!(method.as_str(), "get" | "post" | "put" | "delete" | "patch") {
                            let summary = op["summary"].as_str().unwrap_or(path.as_str());
                            let slug = format!(
                                "{}-{}",
                                method,
                                path.trim_start_matches('/')
                                    .replace('/', "-")
                                    .replace(['{', '}'], "")
                            );
                            let escaped_summary = summary.replace('"', "\\\"");
                            let yaml = format!(
                                "name: {slug}\ndescription: \"{summary}\"\ncommand: curl\nargs: [\"-s\", \"-X\", \"{}\", \"{}{path}\"]\nsafety: readonly\n",
                                method.to_uppercase(), base_url, summary = escaped_summary
                            );
                            tools.push(BundleFile {
                                name: format!("{slug}.yaml"),
                                content: yaml,
                            });
                        }
                    }
                }
            }
        }

        let mut bundle = ExtensionBundle::default();
        bundle.compat_grade = Some(CompatGrade::B);
        bundle.manifest = Some(ExtensionManifest {
            extension: ManifestMeta {
                name: title.to_lowercase().replace(' ', "-"),
                version,
                description: Some(format!("OpenAPI: {}", title)),
                author: None,
                compat: None,
                min_version: None,
                license: None,
                requires: None,
                permissions: None,
            },
        });
        bundle.tools = tools;
        bundle.warnings.push(
            "Tools generated from OpenAPI spec — review commands before approving".to_string(),
        );
        Ok(bundle)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_spec_text;

    #[test]
    fn parse_openapi_json() {
        let raw = r#"{"openapi":"3.0.0","info":{"title":"Demo","version":"1.0.0"},"paths":{}}"#;
        let parsed = parse_spec_text(raw).expect("json should parse");
        assert_eq!(
            parsed.pointer("/info/title").and_then(|v| v.as_str()),
            Some("Demo")
        );
    }

    #[test]
    fn parse_openapi_yaml() {
        let raw = "openapi: 3.0.0\ninfo:\n  title: DemoYaml\n  version: 1.0.0\npaths: {}\n";
        let parsed = parse_spec_text(raw).expect("yaml should parse");
        assert_eq!(
            parsed.pointer("/info/title").and_then(|v| v.as_str()),
            Some("DemoYaml")
        );
    }
}
