use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

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
        Self {
            filename: filename.into(),
            content,
            sha256,
        }
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
        Self {
            skills_dir: skills_dir.into(),
            workflows_dir: workflows_dir.into(),
        }
    }

    /// Build a bundle containing the specified skills (by trigger) and workflows (by name).
    /// Pass empty slices to include all.
    pub fn build(&self, skill_triggers: &[&str], workflow_names: &[&str]) -> Result<Bundle> {
        let skills = self.collect_assets(&self.skills_dir, &["md"], skill_triggers)?;
        let workflows =
            self.collect_assets(&self.workflows_dir, &["yaml", "yml"], workflow_names)?;
        Ok(Bundle::new(skills, workflows))
    }

    fn collect_assets(
        &self,
        dir: &Path,
        exts: &[&str],
        filter: &[&str],
    ) -> Result<Vec<BundleAsset>> {
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut assets = Vec::new();
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            let file_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !exts.contains(&file_ext) {
                continue;
            }
            let Some(filename) = path.file_name().map(|f| f.to_string_lossy().to_string()) else {
                continue;
            };
            if !filter.is_empty() {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                if !filter.iter().any(|f| {
                    stem.trim_start_matches('/') == f.trim_start_matches('/') || *f == stem.as_ref()
                }) {
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
        Self {
            skills_dir: skills_dir.into(),
            workflows_dir: workflows_dir.into(),
        }
    }

    pub fn import(&self, bundle: &Bundle, overwrite: bool) -> Result<Vec<ImportResult>> {
        // verify all hashes first
        for asset in bundle.skills.iter().chain(bundle.workflows.iter()) {
            if !asset.verify() {
                bail!(
                    "hash mismatch for {}: bundle may be corrupted",
                    asset.filename
                );
            }
        }

        let mut results = Vec::new();
        results.extend(self.write_assets(&bundle.skills, &self.skills_dir, overwrite)?);
        results.extend(self.write_assets(&bundle.workflows, &self.workflows_dir, overwrite)?);
        Ok(results)
    }

    fn write_assets(
        &self,
        assets: &[BundleAsset],
        dir: &Path,
        overwrite: bool,
    ) -> Result<Vec<ImportResult>> {
        let _ = std::fs::create_dir_all(dir);
        let mut results = Vec::new();
        for asset in assets {
            let safe_name: String = asset
                .filename
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                        c
                    } else {
                        '-'
                    }
                })
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
            results.push(ImportResult {
                filename: safe_name,
                action,
            });
        }
        Ok(results)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShareArtifactKind {
    MemoryNote,
    RunLearning,
    EvalSummary,
    Custom,
}

impl ShareArtifactKind {
    fn as_str(&self) -> &'static str {
        match self {
            ShareArtifactKind::MemoryNote => "memory-note",
            ShareArtifactKind::RunLearning => "run-learning",
            ShareArtifactKind::EvalSummary => "eval-summary",
            ShareArtifactKind::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareArtifact {
    pub kind: ShareArtifactKind,
    pub artifact_id: String,
    pub summary: String,
    pub payload: String,
    #[serde(default)]
    pub labels: Vec<String>,
}

impl ShareArtifact {
    pub fn new(
        kind: ShareArtifactKind,
        artifact_id: impl Into<String>,
        summary: impl Into<String>,
        payload: impl Into<String>,
        labels: Vec<String>,
    ) -> Self {
        Self {
            kind,
            artifact_id: artifact_id.into(),
            summary: summary.into(),
            payload: payload.into(),
            labels,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharingPolicy {
    pub enabled: bool,
    pub allow_memory_notes: bool,
    pub allow_run_learnings: bool,
    pub allow_eval_summaries: bool,
    #[serde(default)]
    pub deny_labels: Vec<String>,
    #[serde(default)]
    pub redact_keys: Vec<String>,
}

impl Default for SharingPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_memory_notes: true,
            allow_run_learnings: true,
            allow_eval_summaries: true,
            deny_labels: Vec::new(),
            redact_keys: Vec::new(),
        }
    }
}

impl SharingPolicy {
    pub fn collaboration_default() -> Self {
        Self {
            enabled: true,
            allow_memory_notes: true,
            allow_run_learnings: true,
            allow_eval_summaries: true,
            deny_labels: Vec::new(),
            redact_keys: vec![
                "token".to_string(),
                "api_key".to_string(),
                "password".to_string(),
                "secret".to_string(),
            ],
        }
    }

    pub fn allows_kind(&self, kind: &ShareArtifactKind) -> bool {
        if !self.enabled {
            return true;
        }
        match kind {
            ShareArtifactKind::MemoryNote => self.allow_memory_notes,
            ShareArtifactKind::RunLearning => self.allow_run_learnings,
            ShareArtifactKind::EvalSummary => self.allow_eval_summaries,
            ShareArtifactKind::Custom => false,
        }
    }

    pub fn filter_artifact(&self, mut artifact: ShareArtifact) -> SharingDecision {
        if !self.enabled {
            return SharingDecision::allow(artifact, false, "sharing-disabled");
        }

        if !self.allows_kind(&artifact.kind) {
            return SharingDecision::block(format!(
                "artifact-kind-not-allowed:{}",
                artifact.kind.as_str()
            ));
        }

        let deny_labels = self
            .deny_labels
            .iter()
            .map(|label| label.trim().to_ascii_lowercase())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();

        for label in &artifact.labels {
            let normalized = label.trim().to_ascii_lowercase();
            if deny_labels.iter().any(|deny| deny == &normalized) {
                return SharingDecision::block(format!("label-denied:{normalized}"));
            }
        }

        let (payload, redaction_applied) = redact_payload(&artifact.payload, &self.redact_keys);
        artifact.payload = payload;
        SharingDecision::allow(artifact, redaction_applied, "allowed")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharingDecision {
    pub allowed: bool,
    pub reason: String,
    pub redaction_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ShareArtifact>,
}

impl SharingDecision {
    fn allow(artifact: ShareArtifact, redaction_applied: bool, reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            reason: reason.into(),
            redaction_applied,
            artifact: Some(artifact),
        }
    }

    fn block(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: reason.into(),
            redaction_applied: false,
            artifact: None,
        }
    }
}

fn redact_payload(payload: &str, redact_keys: &[String]) -> (String, bool) {
    let mut redacted = payload.to_string();
    let mut changed = false;
    for key in redact_keys {
        let normalized = key.trim();
        if normalized.is_empty() {
            continue;
        }
        let (updated, updated_changed) = redact_key_values(&redacted, normalized);
        redacted = updated;
        changed |= updated_changed;
    }
    (redacted, changed)
}

fn redact_key_values(input: &str, key: &str) -> (String, bool) {
    let marker = format!("{key}=");
    let marker_len = marker.len();
    let lower_marker = marker.to_ascii_lowercase();
    let lower_input = input.to_ascii_lowercase();

    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut changed = false;

    while cursor < input.len() {
        let Some(offset) = lower_input[cursor..].find(&lower_marker) else {
            out.push_str(&input[cursor..]);
            break;
        };

        let start = cursor + offset;
        let value_start = start + marker_len;
        out.push_str(&input[cursor..value_start]);

        let value_end = find_value_end(input, value_start);
        if value_end > value_start {
            out.push_str("[redacted]");
            changed = true;
        }
        cursor = value_end;
    }

    (out, changed)
}

fn find_value_end(input: &str, from: usize) -> usize {
    let bytes = input.as_bytes();
    let mut index = from;
    while index < bytes.len() {
        let b = bytes[index];
        if b.is_ascii_whitespace() || matches!(b, b',' | b';' | b'&' | b'|') {
            break;
        }
        index += 1;
    }
    index
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
            vec![BundleAsset::new(
                "skill.md",
                "---\ntrigger: /test\n---\nhello",
            )],
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

    #[test]
    fn policy_blocks_disallowed_kind() {
        let policy = SharingPolicy {
            enabled: true,
            allow_memory_notes: true,
            allow_run_learnings: false,
            allow_eval_summaries: true,
            deny_labels: vec![],
            redact_keys: vec![],
        };

        let artifact = ShareArtifact::new(
            ShareArtifactKind::RunLearning,
            "run:42",
            "learning",
            "status=ok",
            vec!["learning".to_string()],
        );

        let decision = policy.filter_artifact(artifact);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "artifact-kind-not-allowed:run-learning");
        assert!(decision.artifact.is_none());
    }

    #[test]
    fn policy_blocks_denied_label() {
        let policy = SharingPolicy {
            enabled: true,
            allow_memory_notes: true,
            allow_run_learnings: true,
            allow_eval_summaries: true,
            deny_labels: vec!["internal-only".to_string()],
            redact_keys: vec![],
        };

        let artifact = ShareArtifact::new(
            ShareArtifactKind::MemoryNote,
            "memory:1",
            "memory",
            "owner=neo",
            vec!["internal-only".to_string()],
        );

        let decision = policy.filter_artifact(artifact);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "label-denied:internal-only");
    }

    #[test]
    fn policy_redacts_sensitive_values() {
        let policy = SharingPolicy::collaboration_default();
        let artifact = ShareArtifact::new(
            ShareArtifactKind::MemoryNote,
            "memory:1",
            "memory",
            "owner=neo token=abc123 status=ok",
            vec!["memory".to_string()],
        );

        let decision = policy.filter_artifact(artifact);
        assert!(decision.allowed);
        assert!(decision.redaction_applied);
        let payload = &decision.artifact.expect("artifact").payload;
        assert_eq!(payload, "owner=neo token=[redacted] status=ok");
    }
}
