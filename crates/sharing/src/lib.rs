use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A single skill or workflow packed into a bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleAsset {
    /// Relative path inside the target skills/ or workflows/ directory.
    /// For backward compatibility this field keeps the historical name `filename`.
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
    #[serde(default)]
    pub tools: Vec<BundleAsset>,
}

impl Bundle {
    pub fn new(
        skills: Vec<BundleAsset>,
        workflows: Vec<BundleAsset>,
        tools: Vec<BundleAsset>,
    ) -> Self {
        Self {
            version: "1".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            skills,
            workflows,
            tools,
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
    tools_dir: PathBuf,
}

impl BundleBuilder {
    pub fn new(skills_dir: impl Into<PathBuf>, workflows_dir: impl Into<PathBuf>) -> Self {
        let skills_dir = skills_dir.into();
        let workflows_dir = workflows_dir.into();
        let home = skills_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            skills_dir,
            workflows_dir,
            tools_dir: home.join("tools"),
        }
    }

    /// Build a bundle containing the specified skills (by trigger) and workflows (by name).
    /// Pass empty slices to include all.
    pub fn build(&self, skill_triggers: &[&str], workflow_names: &[&str]) -> Result<Bundle> {
        let skills = self.collect_skill_assets(skill_triggers)?;
        let workflows = self.collect_workflow_assets(workflow_names)?;
        let tools = self.collect_tools_assets()?;
        Ok(Bundle::new(skills, workflows, tools))
    }

    fn collect_skill_assets(&self, filter: &[&str]) -> Result<Vec<BundleAsset>> {
        if !self.skills_dir.exists() {
            return Ok(Vec::new());
        }
        let normalized_filter = normalize_skill_filter(filter);
        let mut assets: Vec<BundleAsset> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for entry in std::fs::read_dir(&self.skills_dir)?.flatten() {
            let path = entry.path();
            if path.is_file() {
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let content = std::fs::read_to_string(&path)?;
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                let trigger =
                    parse_frontmatter_value(&content, "trigger").map(|v| normalize_trigger_key(&v));
                if !filter_matches_skill(&normalized_filter, &stem, trigger.as_deref()) {
                    continue;
                }
                let rel = path
                    .strip_prefix(&self.skills_dir)
                    .ok()
                    .and_then(|p| p.to_str())
                    .unwrap_or_default()
                    .replace('\\', "/");
                if !rel.is_empty() && seen.insert(rel.clone()) {
                    assets.push(BundleAsset::new(rel, content));
                }
                continue;
            }

            if !path.is_dir() {
                continue;
            }
            let manifest = path.join("SKILL.md");
            if !manifest.is_file() {
                continue;
            }
            let manifest_content = std::fs::read_to_string(&manifest)?;
            let package_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let trigger = parse_frontmatter_value(&manifest_content, "trigger")
                .map(|v| normalize_trigger_key(&v));
            if !filter_matches_skill(&normalized_filter, &package_name, trigger.as_deref()) {
                continue;
            }
            collect_files_recursive(&path, &self.skills_dir, &mut assets, &mut seen)?;
        }
        Ok(assets)
    }

    fn collect_workflow_assets(&self, filter: &[&str]) -> Result<Vec<BundleAsset>> {
        if !self.workflows_dir.exists() {
            return Ok(Vec::new());
        }
        let normalized_filter: HashSet<String> = filter
            .iter()
            .map(|s| s.trim().trim_start_matches('/').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let mut assets: Vec<BundleAsset> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for entry in std::fs::read_dir(&self.workflows_dir)?.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            if ext != "yaml" && ext != "yml" {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !normalized_filter.is_empty() && !normalized_filter.contains(&stem) {
                continue;
            }
            let rel = path
                .strip_prefix(&self.workflows_dir)
                .ok()
                .and_then(|p| p.to_str())
                .unwrap_or_default()
                .replace('\\', "/");
            if rel.is_empty() {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            if seen.insert(rel.clone()) {
                assets.push(BundleAsset::new(rel, content));
            }
        }
        Ok(assets)
    }

    fn collect_tools_assets(&self) -> Result<Vec<BundleAsset>> {
        if !self.tools_dir.exists() {
            return Ok(Vec::new());
        }
        let mut assets = Vec::new();
        let mut seen = HashSet::new();
        collect_files_recursive(&self.tools_dir, &self.tools_dir, &mut assets, &mut seen)?;
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
    tools_dir: PathBuf,
}

impl BundleImporter {
    pub fn new(skills_dir: impl Into<PathBuf>, workflows_dir: impl Into<PathBuf>) -> Self {
        let skills_dir = skills_dir.into();
        let workflows_dir = workflows_dir.into();
        let home = skills_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            skills_dir,
            workflows_dir,
            tools_dir: home.join("tools"),
        }
    }

    pub fn import(&self, bundle: &Bundle, overwrite: bool) -> Result<Vec<ImportResult>> {
        // verify all hashes first
        for asset in bundle
            .skills
            .iter()
            .chain(bundle.workflows.iter())
            .chain(bundle.tools.iter())
        {
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
        results.extend(self.write_assets(&bundle.tools, &self.tools_dir, overwrite)?);
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
            let safe_name = sanitize_relative_asset_path(&asset.filename)?;
            let dest = dir.join(&safe_name);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
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
                filename: safe_name.to_string_lossy().to_string(),
                action,
            });
        }
        Ok(results)
    }
}

fn normalize_trigger_key(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn normalize_skill_filter(filter: &[&str]) -> HashSet<String> {
    filter
        .iter()
        .map(|item| item.trim().to_ascii_lowercase())
        .flat_map(|item| {
            let stripped = item.trim_start_matches('/').to_string();
            if stripped != item {
                vec![item, stripped]
            } else {
                vec![item]
            }
        })
        .filter(|item| !item.is_empty())
        .collect()
}

fn filter_matches_skill(
    filter: &HashSet<String>,
    file_stem_or_dir: &str,
    trigger_key: Option<&str>,
) -> bool {
    if filter.is_empty() {
        return true;
    }
    let stem_key = file_stem_or_dir.trim().to_ascii_lowercase();
    if filter.contains(&stem_key) {
        return true;
    }
    if let Some(trigger_key) = trigger_key {
        return filter.contains(trigger_key);
    }
    false
}

fn parse_frontmatter_value(content: &str, key: &str) -> Option<String> {
    let body = content.strip_prefix("---")?;
    let end = body.find("\n---")?;
    let yaml = &body[..end];
    for line in yaml.lines() {
        let mut split = line.splitn(2, ':');
        let left = split.next()?.trim();
        if left != key {
            continue;
        }
        return split.next().map(|right| right.trim().to_string());
    }
    None
}

fn collect_files_recursive(
    root: &Path,
    base_dir: &Path,
    out: &mut Vec<BundleAsset>,
    seen: &mut HashSet<String>,
) -> Result<()> {
    for entry in std::fs::read_dir(root)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, base_dir, out, seen)?;
            continue;
        }
        let rel = path
            .strip_prefix(base_dir)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or_default()
            .replace('\\', "/");
        if rel.is_empty() || !seen.insert(rel.clone()) {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        out.push(BundleAsset::new(rel, content));
    }
    Ok(())
}

fn sanitize_relative_asset_path(raw: &str) -> Result<PathBuf> {
    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        bail!("asset path must be relative: {}", raw);
    }
    let mut out = PathBuf::new();
    for segment in normalized.split('/') {
        let seg = segment.trim();
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            bail!("asset path contains parent traversal: {}", raw);
        }
        let safe: String = seg
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        if safe.is_empty() {
            continue;
        }
        out.push(safe);
    }
    if out.as_os_str().is_empty() {
        bail!("asset path is empty after sanitization: {}", raw);
    }
    Ok(out)
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

        let bundle = Bundle::new(vec![asset], vec![], vec![]);
        let importer = BundleImporter::new(&skills_dir, dir.path().join("workflows"));
        let results = importer.import(&bundle, false).unwrap();
        assert_eq!(results[0].action, ImportAction::Skipped);
    }

    #[test]
    fn importer_preserves_nested_paths() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = Bundle::new(
            vec![
                BundleAsset::new(
                    "review-skill/SKILL.md",
                    "---\nname: review\ndescription: test\ntrigger: /review\n---\nbody",
                ),
                BundleAsset::new(
                    "review-skill/tools/analyze.sh",
                    "#!/usr/bin/env bash\necho ok",
                ),
            ],
            vec![],
            vec![],
        );
        let importer = BundleImporter::new(dir.path().join("skills"), dir.path().join("workflows"));
        let results = importer.import(&bundle, true).unwrap();
        assert_eq!(results.len(), 2);
        assert!(dir
            .path()
            .join("skills")
            .join("review-skill")
            .join("SKILL.md")
            .exists());
        assert!(dir
            .path()
            .join("skills")
            .join("review-skill")
            .join("tools")
            .join("analyze.sh")
            .exists());
    }

    #[test]
    fn importer_restores_project_tools() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = Bundle::new(
            vec![],
            vec![],
            vec![BundleAsset::new(
                "adb/flash.sh",
                "#!/usr/bin/env bash\necho flash",
            )],
        );
        let importer = BundleImporter::new(dir.path().join("skills"), dir.path().join("workflows"));
        importer.import(&bundle, true).unwrap();
        assert!(dir
            .path()
            .join("tools")
            .join("adb")
            .join("flash.sh")
            .exists());
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
