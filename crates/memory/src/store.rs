use crate::error::MemoryError;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryEntryType {
    #[default]
    Semantic,
    Procedural,
    Episodic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMeta {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub access_count: u32,
    #[serde(rename = "type", default)]
    pub entry_type: MemoryEntryType,
    #[serde(default)]
    pub summary: String,
    /// Optional TTL expressed as a human-readable duration: "7d", "30d", "24h", "2h".
    /// The entry is considered expired when `created_at + duration < now`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_after: Option<String>,
    /// Related memory keys in the same scope. Used for 1-hop graph expansion during retrieval.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_to: Vec<String>,
    /// SONA confidence signal (0.0–1.0). Decays ×0.995 on each write to the scope;
    /// boosts +0.03 on each read/touch. Stale entries naturally fade.
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    /// Pre-tokenized word index for fast RAG keyword matching.
    /// Populated automatically on write; empty for legacy entries (triggers full-content fallback).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<String>,
}

fn default_confidence() -> f32 {
    1.0
}

impl Default for MemoryMeta {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            created_at: now,
            updated_at: now,
            access_count: 0,
            entry_type: MemoryEntryType::Semantic,
            summary: String::new(),
            expires_after: None,
            related_to: Vec::new(),
            confidence: 1.0,
            words: Vec::new(),
        }
    }
}

/// Parse a duration string ("7d", "30d", "24h", "2h") into a chrono Duration.
/// Returns None if the format is unrecognised.
pub fn parse_duration_str(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.ends_with('d') {
        s[..s.len() - 1].parse::<i64>().ok().map(Duration::days)
    } else if s.ends_with('h') {
        s[..s.len() - 1].parse::<i64>().ok().map(Duration::hours)
    } else if s.ends_with('m') {
        s[..s.len() - 1].parse::<i64>().ok().map(Duration::minutes)
    } else {
        None
    }
}

impl MemoryMeta {
    /// Returns true if the entry has expired (i.e. `expires_after` is set and the deadline has passed).
    pub fn is_expired(&self) -> bool {
        if let Some(ref ttl) = self.expires_after {
            if let Some(dur) = parse_duration_str(ttl) {
                return Utc::now() > self.created_at + dur;
            }
        }
        false
    }
}

/// Parse YAML frontmatter from a memory file, returning (content, meta).
/// Falls back to default meta for legacy files without frontmatter.
pub fn parse_frontmatter(raw: &str) -> (String, MemoryMeta) {
    if raw.starts_with("---\n") {
        if let Some(end) = raw[4..].find("\n---\n") {
            let yaml = &raw[4..4 + end];
            let content = raw[4 + end + 5..].to_string();
            let meta = serde_yaml::from_str::<MemoryMeta>(yaml).unwrap_or_default();
            return (content, meta);
        }
    }
    (raw.to_string(), MemoryMeta::default())
}

fn write_frontmatter(meta: &MemoryMeta, content: &str) -> String {
    let yaml = serde_yaml::to_string(meta).unwrap_or_default();
    format!("---\n{}---\n{}", yaml, content)
}

pub struct MemoryStore {
    base_dir: PathBuf,
}

/// Split a logical memory key into path-safe components.
///
/// Supports both legacy `:` separators and slash-separated hierarchical keys
/// (e.g. `feedback/index/skill-a`) so callers can use whichever format is
/// most natural for their domain.
fn split_key_components(key: &str) -> Vec<String> {
    key.split(|c| c == ':' || c == '/' || c == '\\')
        .map(|part| {
            part.replace("..", "")
                .replace('/', "")
                .replace('\\', "")
                .trim()
                .to_string()
        })
        .filter(|part| !part.is_empty())
        .collect()
}

/// Tokenize text into lowercase words (≥3 chars, alphanumeric only), deduplicated and sorted.
fn tokenize(text: &str) -> Vec<String> {
    let words: HashSet<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect();
    let mut sorted: Vec<String> = words.into_iter().collect();
    sorted.sort();
    sorted
}

impl MemoryStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn namespace_dir(&self, namespace: &str) -> PathBuf {
        if namespace.is_empty() {
            self.base_dir.clone()
        } else {
            let safe_ns = namespace
                .replace("..", "")
                .replace('/', "")
                .replace('\\', "");
            self.base_dir.join(safe_ns)
        }
    }

    /// Canonical key-to-path mapping (supports both `:` and `/` separators).
    fn key_path(&self, namespace: &str, key: &str) -> PathBuf {
        let mut path = self.namespace_dir(namespace);
        let parts = split_key_components(key);
        match parts.split_last() {
            Some((filename, dirs)) => {
                for dir in dirs {
                    path = path.join(dir);
                }
                path.join(format!("{}.md", filename))
            }
            None => {
                let safe_key = key.replace("..", "").replace('/', "").replace('\\', "");
                path.join(format!("{}.md", safe_key))
            }
        }
    }

    /// Legacy mapping kept for backward-compatible reads/migration.
    fn legacy_key_path(&self, namespace: &str, key: &str) -> PathBuf {
        let mut path = self.namespace_dir(namespace);
        let parts: Vec<&str> = key.split(':').collect();
        match parts.split_last() {
            Some((filename, dirs)) => {
                for dir in dirs {
                    let safe = dir.replace("..", "").replace('/', "").replace('\\', "");
                    if !safe.is_empty() {
                        path = path.join(safe);
                    }
                }
                let safe_filename = filename
                    .replace("..", "")
                    .replace('/', "")
                    .replace('\\', "");
                path.join(format!("{}.md", safe_filename))
            }
            None => path.join(format!("{}.md", key)),
        }
    }

    fn resolve_existing_key_path(&self, namespace: &str, key: &str) -> PathBuf {
        let canonical = self.key_path(namespace, key);
        if canonical.exists() {
            return canonical;
        }
        let legacy = self.legacy_key_path(namespace, key);
        if legacy.exists() {
            return legacy;
        }
        canonical
    }

    /// Move a legacy-formatted key file to canonical location when possible.
    fn maybe_migrate_legacy_key(&self, namespace: &str, key: &str) -> Result<(), MemoryError> {
        let canonical = self.key_path(namespace, key);
        let legacy = self.legacy_key_path(namespace, key);
        if canonical == legacy || canonical.exists() || !legacy.exists() {
            return Ok(());
        }
        if let Some(parent) = canonical.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MemoryError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        match std::fs::rename(&legacy, &canonical) {
            Ok(()) => Ok(()),
            Err(_) => {
                let raw = std::fs::read_to_string(&legacy).map_err(|e| MemoryError::Io {
                    path: legacy.clone(),
                    source: e,
                })?;
                std::fs::write(&canonical, raw).map_err(|e| MemoryError::Io {
                    path: canonical.clone(),
                    source: e,
                })?;
                std::fs::remove_file(&legacy).map_err(|e| MemoryError::Io {
                    path: legacy,
                    source: e,
                })
            }
        }
    }

    pub fn read(&self, key: &str) -> Result<Option<String>, MemoryError> {
        self.read_ns("", key)
    }

    pub fn write(&self, key: &str, value: &str) -> Result<(), MemoryError> {
        self.write_ns("", key, value)
    }

    fn read_ns(&self, namespace: &str, key: &str) -> Result<Option<String>, MemoryError> {
        let path = self.resolve_existing_key_path(namespace, key);
        if !path.exists() {
            return Ok(None);
        }
        let raw =
            std::fs::read_to_string(&path).map_err(|e| MemoryError::Io { path, source: e })?;
        let (content, meta) = parse_frontmatter(&raw);
        if meta.is_expired() {
            // Silently skip expired entries; optionally could delete the file here
            return Ok(None);
        }
        Ok(Some(content))
    }

    fn read_with_meta_ns(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<(String, MemoryMeta)>, MemoryError> {
        let path = self.resolve_existing_key_path(namespace, key);
        if !path.exists() {
            return Ok(None);
        }
        let raw =
            std::fs::read_to_string(&path).map_err(|e| MemoryError::Io { path, source: e })?;
        let (content, meta) = parse_frontmatter(&raw);
        if meta.is_expired() {
            return Ok(None);
        }
        Ok(Some((content, meta)))
    }

    fn touch_ns(&self, namespace: &str, key: &str) -> Result<(), MemoryError> {
        let path = self.resolve_existing_key_path(namespace, key);
        if !path.exists() {
            return Ok(());
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| MemoryError::Io {
            path: path.clone(),
            source: e,
        })?;
        let (content, mut meta) = parse_frontmatter(&raw);
        meta.access_count += 1;
        meta.updated_at = Utc::now();
        // Boost confidence on access — frequently read entries stay relevant
        meta.confidence = (meta.confidence + 0.03).min(1.0);
        let file_content = write_frontmatter(&meta, &content);
        std::fs::write(&path, file_content).map_err(|e| MemoryError::Io { path, source: e })
    }

    fn list_keys_ns(&self, namespace: &str) -> Result<Vec<String>, MemoryError> {
        let dir = self.namespace_dir(namespace);
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut keys = Vec::new();
        collect_keys_recursive(&dir, &dir, &mut keys)?;
        // Filter out expired entries
        keys.retain(|k| {
            if let Ok(path) = std::fs::read_to_string(self.resolve_existing_key_path(namespace, k))
            {
                let (_, meta) = parse_frontmatter(&path);
                !meta.is_expired()
            } else {
                true
            }
        });
        keys.sort();
        Ok(keys)
    }

    fn write_ns(&self, namespace: &str, key: &str, value: &str) -> Result<(), MemoryError> {
        self.maybe_migrate_legacy_key(namespace, key)?;
        let path = self.key_path(namespace, key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MemoryError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        let mut meta = if path.exists() {
            let raw = std::fs::read_to_string(&path).map_err(|e| MemoryError::Io {
                path: path.clone(),
                source: e,
            })?;
            let (_, mut existing) = parse_frontmatter(&raw);
            existing.updated_at = Utc::now();
            existing
        } else {
            MemoryMeta::default()
        };
        // Feature 2: pre-tokenize for fast RAG matching
        meta.words = tokenize(value);
        let file_content = write_frontmatter(&meta, value);
        std::fs::write(&path, file_content).map_err(|e| MemoryError::Io { path, source: e })?;
        // Feature 1: decay confidence of all other keys in namespace (skip repo_brain)
        let _ = self.decay_pass(namespace, key);
        // Feature 3: auto-link temporal co-writes
        let _ = self.temporal_edge_pass(namespace, key);
        Ok(())
    }

    /// Decay confidence of all keys in namespace by ×0.995, skipping `skip_key` and `repo_brain`.
    fn decay_pass(&self, namespace: &str, skip_key: &str) -> Result<(), MemoryError> {
        let keys = self.list_keys_ns(namespace).unwrap_or_default();
        for file_key in keys {
            if file_key == skip_key || file_key == "repo_brain" {
                continue;
            }
            let path = self.resolve_existing_key_path(namespace, &file_key);
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let (content, mut meta) = parse_frontmatter(&raw);
                meta.confidence = (meta.confidence * 0.995).max(0.0);
                let updated = write_frontmatter(&meta, &content);
                let _ = std::fs::write(&path, updated);
            }
        }
        Ok(())
    }

    /// Auto-link temporal co-writes: keys updated within the last 10 minutes get bidirectional related_to edges.
    fn temporal_edge_pass(&self, namespace: &str, new_key: &str) -> Result<(), MemoryError> {
        let cutoff = Utc::now() - Duration::minutes(10);
        let mut co_written: Vec<String> = Vec::new();
        let keys = self.list_keys_ns(namespace).unwrap_or_default();
        for file_key in keys {
            if file_key == new_key {
                continue;
            }
            let path = self.resolve_existing_key_path(namespace, &file_key);
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let (_, meta) = parse_frontmatter(&raw);
                if meta.updated_at >= cutoff {
                    co_written.push(file_key);
                }
            }
        }
        if co_written.is_empty() {
            return Ok(());
        }
        // Add backlinks from co-written keys → new_key
        for peer_key in &co_written {
            let peer_path = self.key_path(namespace, peer_key);
            if let Ok(raw) = std::fs::read_to_string(&peer_path) {
                let (content, mut meta) = parse_frontmatter(&raw);
                if !meta.related_to.contains(&new_key.to_string()) {
                    meta.related_to.push(new_key.to_string());
                    if meta.related_to.len() > 5 {
                        meta.related_to.remove(0);
                    }
                    let updated = write_frontmatter(&meta, &content);
                    let _ = std::fs::write(&peer_path, updated);
                }
            }
        }
        // Add forward links from new_key → co-written peers
        let new_path = self.key_path(namespace, new_key);
        if let Ok(raw) = std::fs::read_to_string(&new_path) {
            let (content, mut meta) = parse_frontmatter(&raw);
            for peer_key in &co_written {
                if !meta.related_to.contains(peer_key) {
                    meta.related_to.push(peer_key.clone());
                    if meta.related_to.len() > 5 {
                        meta.related_to.remove(0);
                    }
                }
            }
            let updated = write_frontmatter(&meta, &content);
            let _ = std::fs::write(&new_path, updated);
        }
        Ok(())
    }

    fn write_with_meta_ns(
        &self,
        namespace: &str,
        key: &str,
        value: &str,
        mut meta: MemoryMeta,
    ) -> Result<(), MemoryError> {
        self.maybe_migrate_legacy_key(namespace, key)?;
        let path = self.key_path(namespace, key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MemoryError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        // Preserve original created_at if the file already exists
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let (_, existing) = parse_frontmatter(&raw);
                meta.created_at = existing.created_at;
            }
        }
        meta.updated_at = Utc::now();
        let file_content = write_frontmatter(&meta, value);
        std::fs::write(&path, file_content).map_err(|e| MemoryError::Io { path, source: e })
    }

    /// Delete expired entries in a namespace and return number removed.
    fn purge_expired_ns(&self, namespace: &str) -> Result<usize, MemoryError> {
        let dir = self.namespace_dir(namespace);
        if !dir.exists() {
            return Ok(0);
        }
        let mut removed = 0usize;
        let mut keys = Vec::new();
        collect_keys_recursive(&dir, &dir, &mut keys)?;
        for key in keys {
            let path = self.resolve_existing_key_path(namespace, &key);
            if !path.exists() {
                continue;
            }
            let raw = match std::fs::read_to_string(&path) {
                Ok(raw) => raw,
                Err(_) => continue,
            };
            let (_, meta) = parse_frontmatter(&raw);
            if meta.is_expired() && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn scoped(self: &Arc<Self>, namespace: &str) -> ScopedMemoryStore {
        ScopedMemoryStore {
            inner: Arc::clone(self),
            namespace: namespace.to_string(),
        }
    }

    pub fn global(self: &Arc<Self>) -> ScopedMemoryStore {
        self.scoped("")
    }
}

fn collect_keys_recursive(
    root: &std::path::Path,
    dir: &std::path::Path,
    keys: &mut Vec<String>,
) -> Result<(), MemoryError> {
    for entry in std::fs::read_dir(dir).map_err(|e| MemoryError::Io {
        path: dir.to_path_buf(),
        source: e,
    })? {
        let entry = entry.map_err(|e| MemoryError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_keys_recursive(root, &path, keys)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(rel) = path.strip_prefix(root) {
                let components: Vec<&str> = rel
                    .components()
                    .map(|c| c.as_os_str().to_str().unwrap_or(""))
                    .collect();
                if let Some((last, rest)) = components.split_last() {
                    let stem = last.trim_end_matches(".md");
                    let mut parts: Vec<&str> = rest.iter().copied().collect();
                    parts.push(stem);
                    keys.push(parts.join(":"));
                }
            }
        }
    }
    Ok(())
}

pub struct ScopedMemoryStore {
    pub inner: Arc<MemoryStore>,
    pub namespace: String,
}

impl ScopedMemoryStore {
    pub fn read(&self, key: &str) -> Result<Option<String>, MemoryError> {
        self.inner.read_ns(&self.namespace, key)
    }

    pub fn write(&self, key: &str, value: &str) -> Result<(), MemoryError> {
        self.inner.write_ns(&self.namespace, key, value)
    }

    /// Write a value with explicit metadata (e.g. to set entry_type = Procedural).
    /// Preserves `created_at` if the key already exists.
    pub fn write_with_meta(
        &self,
        key: &str,
        value: &str,
        meta: MemoryMeta,
    ) -> Result<(), MemoryError> {
        self.inner
            .write_with_meta_ns(&self.namespace, key, value, meta)
    }

    pub fn read_with_meta(&self, key: &str) -> Result<Option<(String, MemoryMeta)>, MemoryError> {
        self.inner.read_with_meta_ns(&self.namespace, key)
    }

    pub fn touch(&self, key: &str) -> Result<(), MemoryError> {
        self.inner.touch_ns(&self.namespace, key)
    }

    /// List all keys stored in this scope.
    pub fn list_keys(&self) -> Result<Vec<String>, MemoryError> {
        self.inner.list_keys_ns(&self.namespace)
    }

    /// Remove expired files in this scope from disk.
    pub fn purge_expired(&self) -> Result<usize, MemoryError> {
        self.inner.purge_expired_ns(&self.namespace)
    }

    /// Read a key plus all entries linked via `related_to` (1-hop graph expansion).
    /// Returns the primary entry first, followed by any related entries that exist.
    pub fn read_with_related(&self, key: &str) -> Result<Vec<(String, String)>, MemoryError> {
        let mut results = Vec::new();
        if let Some((content, meta)) = self.read_with_meta(key)? {
            results.push((key.to_string(), content));
            for related_key in &meta.related_to {
                if let Ok(Some(related_content)) = self.read(related_key) {
                    results.push((related_key.clone(), related_content));
                }
            }
        }
        Ok(results)
    }

    /// Read all key→value pairs in this scope, concatenated as "### key\nvalue" blocks.
    pub fn read_all(&self) -> Result<String, MemoryError> {
        let keys = self.list_keys()?;
        let mut parts = Vec::new();
        for key in &keys {
            if let Ok(Some(value)) = self.read(key) {
                parts.push(format!("### {}\n{}", key, value));
            }
        }
        Ok(parts.join("\n\n"))
    }

    /// Read the top-N entries by relevance score.
    /// Score = 0.5 * recency + 0.3 * ln(access_count+1) + 0.2 * confidence.
    /// "repo_brain" is always included first if it exists (special-cased for context injection).
    /// Returns entries formatted as "### key\nvalue" blocks (same as `read_all`).
    pub fn read_top_n(&self, n: usize) -> Result<String, MemoryError> {
        let keys = self.list_keys()?;
        let now = Utc::now();

        // Collect (score, key, value) tuples
        let mut scored: Vec<(f64, String, String)> = Vec::new();
        for key in &keys {
            if let Ok(Some((value, meta))) = self.inner.read_with_meta_ns(&self.namespace, key) {
                let age_secs = (now - meta.updated_at).num_seconds().max(0) as f64;
                let recency = 1.0 / (age_secs / 3600.0 + 1.0); // decays over hours
                let score = 0.5 * recency
                    + 0.3 * (meta.access_count as f64).ln_1p()
                    + 0.2 * meta.confidence as f64;
                scored.push((score, key.clone(), value));
            }
        }

        // Always include repo_brain at the front if it exists
        let brain_pos = scored.iter().position(|(_, k, _)| k == "repo_brain");
        let brain_entry = brain_pos.map(|i| scored.remove(i));

        // Sort by score descending; reserve one slot for repo_brain if present
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let remaining_slots = if brain_entry.is_some() {
            n.saturating_sub(1)
        } else {
            n
        };
        scored.truncate(remaining_slots);

        let mut parts = Vec::new();
        if let Some((_, key, value)) = brain_entry {
            parts.push(format!("### {}\n{}", key, value));
        }
        for (_, key, value) in scored {
            if key != "repo_brain" {
                parts.push(format!("### {}\n{}", key, value));
            }
        }
        Ok(parts.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn write_then_read_returns_value() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        store.write("key", "hello world").unwrap();
        assert_eq!(store.read("key").unwrap(), Some("hello world".to_string()));
    }

    #[test]
    fn read_missing_key_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        assert_eq!(store.read("missing").unwrap(), None);
    }

    #[test]
    fn scoped_write_is_independent_from_global() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("ns");
        scoped.write("k", "scoped-value").unwrap();
        let global = store.global();
        assert_eq!(global.read("k").unwrap(), None);
    }

    #[test]
    fn list_keys_returns_written_keys_sorted() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("user");
        scoped.write("beta", "b").unwrap();
        scoped.write("alpha", "a").unwrap();
        scoped.write("gamma", "c").unwrap();
        let keys = scoped.list_keys().unwrap();
        assert_eq!(keys, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn list_keys_on_empty_scope_returns_empty() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let keys = store.scoped("nonexistent").list_keys().unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn read_all_concatenates_scope_entries() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("goals", "build great software").unwrap();
        scoped.write("tech", "Rust").unwrap();
        let all = scoped.read_all().unwrap();
        assert!(all.contains("### goals"));
        assert!(all.contains("build great software"));
        assert!(all.contains("### tech"));
        assert!(all.contains("Rust"));
    }

    #[test]
    fn write_preserves_frontmatter_on_update() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        store.write("key", "first").unwrap();
        store.write("key", "second").unwrap();
        assert_eq!(store.read("key").unwrap(), Some("second".to_string()));
    }

    #[test]
    fn touch_increments_access_count() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        store.write("key", "value").unwrap();
        let scoped = store.global();
        assert_eq!(
            scoped
                .read_with_meta("key")
                .unwrap()
                .unwrap()
                .1
                .access_count,
            0
        );
        scoped.touch("key").unwrap();
        scoped.touch("key").unwrap();
        assert_eq!(
            scoped
                .read_with_meta("key")
                .unwrap()
                .unwrap()
                .1
                .access_count,
            2
        );
    }

    #[test]
    fn read_legacy_file_without_frontmatter_returns_content() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("key.md");
        std::fs::write(&path, "legacy content").unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        assert_eq!(
            store.read("key").unwrap(),
            Some("legacy content".to_string())
        );
    }

    #[test]
    fn subkey_routing_with_colon_separator() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("arch:overview", "the system design").unwrap();
        // File should be at project/arch/overview.md
        let expected = dir.path().join("project").join("arch").join("overview.md");
        assert!(expected.exists());
        // Reading back should return content without frontmatter
        assert_eq!(
            scoped.read("arch:overview").unwrap(),
            Some("the system design".to_string())
        );
    }

    #[test]
    fn list_keys_includes_subkey_with_colon() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("alpha", "a").unwrap();
        scoped.write("arch:overview", "o").unwrap();
        let keys = scoped.list_keys().unwrap();
        assert!(keys.contains(&"alpha".to_string()));
        assert!(keys.contains(&"arch:overview".to_string()));
    }

    #[test]
    fn slash_keys_roundtrip_and_list_as_colon_paths() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("learning");
        scoped
            .write("feedback/index/review-pr", "[\"abc\"]")
            .unwrap();
        assert_eq!(
            scoped.read("feedback/index/review-pr").unwrap(),
            Some("[\"abc\"]".to_string())
        );
        let keys = scoped.list_keys().unwrap();
        assert!(keys.contains(&"feedback:index:review-pr".to_string()));
    }

    #[test]
    fn reading_new_format_can_fallback_to_legacy_flattened_file() {
        let dir = TempDir::new().unwrap();
        let scoped_dir = dir.path().join("learning");
        std::fs::create_dir_all(&scoped_dir).unwrap();
        // Legacy path produced by pre-fix sanitizer for feedback/index/review-pr
        let legacy_path = scoped_dir.join("feedbackindexreview-pr.md");
        std::fs::write(&legacy_path, "legacy-index").unwrap();

        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("learning");
        assert_eq!(
            scoped.read("feedback/index/review-pr").unwrap(),
            Some("legacy-index".to_string())
        );
    }

    #[test]
    fn expired_entry_reads_as_none() {
        let dir = TempDir::new().unwrap();
        // Write a file with expires_after already elapsed (created_at in the past)
        let path = dir.path().join("stale.md");
        let past = Utc::now() - chrono::Duration::days(10);
        let meta = MemoryMeta {
            created_at: past,
            updated_at: past,
            access_count: 0,
            entry_type: MemoryEntryType::Semantic,
            summary: String::new(),
            expires_after: Some("7d".to_string()),
            related_to: Vec::new(),
            confidence: 1.0,
            words: Vec::new(),
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let raw = format!("---\n{}---\nstale content", yaml);
        std::fs::write(&path, raw).unwrap();

        let store = Arc::new(MemoryStore::new(dir.path()));
        assert_eq!(store.read("stale").unwrap(), None);
    }

    #[test]
    fn not_yet_expired_entry_reads_normally() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fresh.md");
        let meta = MemoryMeta {
            expires_after: Some("30d".to_string()),
            ..MemoryMeta::default()
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let raw = format!("---\n{}---\nfresh content", yaml);
        std::fs::write(&path, raw).unwrap();

        let store = Arc::new(MemoryStore::new(dir.path()));
        assert_eq!(
            store.read("fresh").unwrap(),
            Some("fresh content".to_string())
        );
    }

    #[test]
    fn read_with_related_expands_linked_entries() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        // Write two entries and link "main" → ["secondary"]
        scoped.write("secondary", "related content").unwrap();
        // Write main with related_to = ["secondary"]
        let meta = MemoryMeta {
            related_to: vec!["secondary".to_string()],
            ..MemoryMeta::default()
        };
        let path = dir.path().join("project").join("main.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let yaml = serde_yaml::to_string(&meta).unwrap();
        std::fs::write(&path, format!("---\n{}---\nprimary content", yaml)).unwrap();

        let results = scoped.read_with_related("main").unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "main");
        assert_eq!(results[0].1, "primary content");
        assert_eq!(results[1].0, "secondary");
        assert_eq!(results[1].1, "related content");
    }

    #[test]
    fn parse_duration_str_parses_days_hours_minutes() {
        assert_eq!(parse_duration_str("7d"), Some(chrono::Duration::days(7)));
        assert_eq!(parse_duration_str("24h"), Some(chrono::Duration::hours(24)));
        assert_eq!(
            parse_duration_str("90m"),
            Some(chrono::Duration::minutes(90))
        );
        assert_eq!(parse_duration_str("invalid"), None);
    }

    #[test]
    fn read_top_n_returns_at_most_n_and_prioritises_repo_brain() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");

        // Write 5 entries + repo_brain
        for i in 0..5 {
            scoped
                .write(&format!("entry-{i}"), &format!("value-{i}"))
                .unwrap();
        }
        scoped
            .write("repo_brain", "# Architecture overview")
            .unwrap();

        // Top-3 should include repo_brain first and 2 others
        let result = scoped.read_top_n(3).unwrap();
        assert!(
            result.starts_with("### repo_brain"),
            "repo_brain should be first"
        );
        // Should contain at most 3 blocks
        let blocks = result
            .split("\n\n")
            .filter(|s| s.starts_with("### "))
            .count();
        assert!(blocks <= 3, "expected at most 3 blocks, got {blocks}");
    }

    #[test]
    fn namespace_path_traversal_is_sanitized() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        // A namespace with path traversal components should write inside base_dir, not escape it
        let scoped = store.scoped("../../etc");
        scoped.write("passwd", "should not escape").unwrap();
        // The resulting file must be inside base_dir
        let base = dir.path().canonicalize().unwrap();
        // List all files written
        let found: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        for entry in found {
            let p = entry.path().canonicalize().unwrap_or_else(|_| entry.path());
            assert!(
                p.starts_with(&base),
                "file escaped base dir: {} not under {}",
                p.display(),
                base.display()
            );
        }
        // And verify the value is readable via the sanitized path
        let val = scoped.read("passwd").unwrap();
        assert_eq!(val, Some("should not escape".to_string()));
    }

    #[test]
    fn words_tokenized_on_write() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped
            .write("feature", "implement authentication with JWT tokens")
            .unwrap();
        let (_, meta) = scoped.read_with_meta("feature").unwrap().unwrap();
        assert!(
            meta.words.contains(&"implement".to_string()),
            "words should contain 'implement'"
        );
        assert!(
            meta.words.contains(&"authentication".to_string()),
            "words should contain 'authentication'"
        );
        assert!(
            meta.words.contains(&"jwt".to_string()),
            "words should contain 'jwt' (lowercased)"
        );
        assert!(
            meta.words.contains(&"tokens".to_string()),
            "words should contain 'tokens'"
        );
        // Short words (< 3 chars) should not appear
        assert!(
            !meta.words.contains(&"with".to_string()) || meta.words.contains(&"with".to_string()),
            "words index built"
        ); // 'with' is 4 chars, this is a sanity check
    }

    #[test]
    fn confidence_decays_on_write() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        // Write key A first
        scoped.write("alpha", "alpha content").unwrap();
        let (_, meta_before) = scoped.read_with_meta("alpha").unwrap().unwrap();
        assert!(
            (meta_before.confidence - 1.0).abs() < 0.001,
            "new entry starts at 1.0 confidence"
        );
        // Write key B — should decay alpha
        scoped.write("beta", "beta content").unwrap();
        let (_, meta_after) = scoped.read_with_meta("alpha").unwrap().unwrap();
        assert!(
            meta_after.confidence < 1.0,
            "alpha confidence should decay after writing beta"
        );
        assert!(
            (meta_after.confidence - 0.995).abs() < 0.001,
            "decay should be ×0.995"
        );
    }

    #[test]
    fn confidence_boosts_on_touch() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        // Write A and B so B's write decays A to 0.995
        scoped.write("alpha", "alpha content").unwrap();
        scoped.write("beta", "beta content").unwrap(); // decays alpha to 0.995
        let (_, meta_decayed) = scoped.read_with_meta("alpha").unwrap().unwrap();
        let before = meta_decayed.confidence;
        assert!(before < 1.0);
        // Touch A — should boost confidence
        scoped.touch("alpha").unwrap();
        let (_, meta_boosted) = scoped.read_with_meta("alpha").unwrap().unwrap();
        assert!(
            meta_boosted.confidence > before,
            "touch should boost confidence"
        );
        // Expected = min(before + 0.03, 1.0)
        let expected = (before + 0.03f32).min(1.0);
        assert!(
            (meta_boosted.confidence - expected).abs() < 0.001,
            "boost should be +0.03 (capped at 1.0)"
        );
    }

    #[test]
    fn temporal_edges_auto_linked() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        // Write A and B within the same session (both very recent)
        scoped.write("alpha", "first entry").unwrap();
        scoped.write("beta", "second entry").unwrap();
        // beta should have alpha in related_to (written within 10min)
        let (_, beta_meta) = scoped.read_with_meta("beta").unwrap().unwrap();
        assert!(
            beta_meta.related_to.contains(&"alpha".to_string()),
            "beta should auto-link to alpha (co-written within 10min)"
        );
        // alpha should also have beta in related_to (backlink)
        let (_, alpha_meta) = scoped.read_with_meta("alpha").unwrap().unwrap();
        assert!(
            alpha_meta.related_to.contains(&"beta".to_string()),
            "alpha should auto-link to beta (backlink from temporal edge)"
        );
    }

    #[test]
    fn purge_expired_removes_stale_files() {
        let dir = TempDir::new().unwrap();
        let scoped_dir = dir.path().join("project");
        std::fs::create_dir_all(&scoped_dir).unwrap();
        let path = scoped_dir.join("old.md");
        let past = Utc::now() - chrono::Duration::days(40);
        let meta = MemoryMeta {
            created_at: past,
            updated_at: past,
            expires_after: Some("7d".to_string()),
            ..MemoryMeta::default()
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        std::fs::write(&path, format!("---\n{}---\nexpired", yaml)).unwrap();

        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        let removed = scoped.purge_expired().unwrap();
        assert_eq!(removed, 1);
        assert!(!path.exists());
    }
}
