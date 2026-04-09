use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A single skill or workflow packed into a bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleAsset {
    pub filename: String,
    pub content: String,
    pub sha256: String,
}

impl BundleAsset {
    pub fn new(filename: impl Into<String>, content: impl Into<String>) -> Self {
        let content = content.into();
        let sha256 = hex::encode(Sha256::digest(content.as_bytes()));
        Self { filename: filename.into(), content, sha256 }
    }

    pub fn verify(&self) -> bool {
        hex::encode(Sha256::digest(self.content.as_bytes())) == self.sha256
    }
}

/// A portable bundle of skills and/or workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub version: String,
    pub created_at: String,
    pub skills: Vec<BundleAsset>,
    pub workflows: Vec<BundleAsset>,
}

impl Bundle {
    pub fn new(skills: Vec<BundleAsset>, workflows: Vec<BundleAsset>) -> Self {
        Self {
            version: "1".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            skills,
            workflows,
        }
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serialize bundle")
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).context("parse bundle")
    }
}

/// Builds a bundle from files on disk.
pub struct BundleBuilder {
    skills_dir: PathBuf,
    workflows_dir: PathBuf,
}

impl BundleBuilder {
    pub fn new(skills_dir: impl Into<PathBuf>, workflows_dir: impl Into<PathBuf>) -> Self {
        Self { skills_dir: skills_dir.into(), workflows_dir: workflows_dir.into() }
    }

    /// Build a bundle containing the specified skills (by trigger) and workflows (by name).
    /// Pass empty slices to include all.
    pub fn build(
        &self,
        skill_triggers: &[&str],
        workflow_names: &[&str],
    ) -> Result<Bundle> {
        let skills = self.collect_assets(&self.skills_dir, &["md"], skill_triggers)?;
        let workflows = self.collect_assets(&self.workflows_dir, &["yaml", "yml"], workflow_names)?;
        Ok(Bundle::new(skills, workflows))
    }

    fn collect_assets(&self, dir: &Path, exts: &[&str], filter: &[&str]) -> Result<Vec<BundleAsset>> {
        if !dir.exists() { return Ok(vec![]); }
        let mut assets = Vec::new();
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            let file_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !exts.contains(&file_ext) { continue; }
            let Some(filename) = path.file_name().map(|f| f.to_string_lossy().to_string()) else {
                continue;
            };
            if !filter.is_empty() {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                if !filter.iter().any(|f| stem.trim_start_matches('/') == f.trim_start_matches('/') || *f == stem.as_ref()) {
                    continue;
                }
            }
            let content = std::fs::read_to_string(&path)?;
            assets.push(BundleAsset::new(filename, content));
        }
        Ok(assets)
    }
}

/// Result of importing one asset.
#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub filename: String,
    pub action: ImportAction,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ImportAction {
    Imported,
    Skipped,
    Overwritten,
}

/// Imports a bundle onto disk.
pub struct BundleImporter {
    skills_dir: PathBuf,
    workflows_dir: PathBuf,
}

impl BundleImporter {
    pub fn new(skills_dir: impl Into<PathBuf>, workflows_dir: impl Into<PathBuf>) -> Self {
        Self { skills_dir: skills_dir.into(), workflows_dir: workflows_dir.into() }
    }

    pub fn import(&self, bundle: &Bundle, overwrite: bool) -> Result<Vec<ImportResult>> {
        // verify all hashes first
        for asset in bundle.skills.iter().chain(bundle.workflows.iter()) {
            if !asset.verify() {
                bail!("hash mismatch for {}: bundle may be corrupted", asset.filename);
            }
        }

        let mut results = Vec::new();
        results.extend(self.write_assets(&bundle.skills, &self.skills_dir, overwrite)?);
        results.extend(self.write_assets(&bundle.workflows, &self.workflows_dir, overwrite)?);
        Ok(results)
    }

    fn write_assets(&self, assets: &[BundleAsset], dir: &Path, overwrite: bool) -> Result<Vec<ImportResult>> {
        let _ = std::fs::create_dir_all(dir);
        let mut results = Vec::new();
        for asset in assets {
            let safe_name: String = asset.filename.chars()
                .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' })
                .collect();
            let dest = dir.join(&safe_name);
            let action = if dest.exists() && !overwrite {
                ImportAction::Skipped
            } else if dest.exists() {
                std::fs::write(&dest, &asset.content)?;
                ImportAction::Overwritten
            } else {
                std::fs::write(&dest, &asset.content)?;
                ImportAction::Imported
            };
            results.push(ImportResult { filename: safe_name, action });
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_asset_hash_roundtrip() {
        let asset = BundleAsset::new("test.md", "hello world");
        assert!(asset.verify());
    }

    #[test]
    fn bundle_asset_detects_tamper() {
        let mut asset = BundleAsset::new("test.md", "hello world");
        asset.content = "tampered".to_string();
        assert!(!asset.verify());
    }

    #[test]
    fn bundle_json_roundtrip() {
        let bundle = Bundle::new(
            vec![BundleAsset::new("skill.md", "---\ntrigger: /test\n---\nhello")],
            vec![],
        );
        let json = bundle.to_json().unwrap();
        let back = Bundle::from_json(&json).unwrap();
        assert_eq!(back.skills[0].filename, "skill.md");
        assert_eq!(back.version, "1");
    }

    #[test]
    fn importer_skips_existing_when_no_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        let asset = BundleAsset::new("existing.md", "original");
        std::fs::write(skills_dir.join("existing.md"), "original").unwrap();

        let bundle = Bundle::new(vec![asset], vec![]);
        let importer = BundleImporter::new(&skills_dir, dir.path().join("workflows"));
        let results = importer.import(&bundle, false).unwrap();
        assert_eq!(results[0].action, ImportAction::Skipped);
    }
}
