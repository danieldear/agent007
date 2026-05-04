use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompatGrade {
    A,
    B,
    C,
}

impl std::fmt::Display for CompatGrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompatGrade::A => write!(f, "A"),
            CompatGrade::B => write!(f, "B"),
            CompatGrade::C => write!(f, "C"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestRequires {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub runtimes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestPermissions {
    pub safety: Option<String>,
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub mcp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestMeta {
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub compat: Option<CompatGrade>,
    pub min_version: Option<String>,
    pub license: Option<String>,
    pub requires: Option<ManifestRequires>,
    pub permissions: Option<ManifestPermissions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub extension: ManifestMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionBundle {
    pub manifest: Option<ExtensionManifest>,
    pub compat_grade: Option<CompatGrade>,
    pub skills: Vec<BundleFile>,
    pub tools: Vec<BundleFile>,
    pub workflows: Vec<BundleFile>,
    pub mcp_servers: Vec<serde_json::Value>,
    pub rag_sources: Vec<serde_json::Value>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleFile {
    pub name: String,
    pub content: String,
}
