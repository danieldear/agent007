use std::path::PathBuf;
use std::sync::Arc;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use crate::error::MemoryError;

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

impl MemoryStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }

    fn key_path(&self, namespace: &str, key: &str) -> PathBuf {
        let mut path = if namespace.is_empty() {
            self.base_dir.clone()
        } else {
            // Sanitize namespace just like key components to prevent path traversal
            let safe_ns = namespace.replace("..", "").replace('/', "").replace('\\', "");
            self.base_dir.join(safe_ns)
        };
        let parts: Vec<&str> = key.split(':').collect();
        match parts.split_last() {
            Some((filename, dirs)) => {
                for dir in dirs {
                    // Sanitize: reject any component that tries to traverse up
                    let safe = dir.replace("..", "").replace('/', "").replace('\\', "");
                    if !safe.is_empty() {
                        path = path.join(safe);
                    }
                }
                let safe_filename = filename.replace("..", "").replace('/', "").replace('\\', "");
                path.join(format!("{}.md", safe_filename))
            }
            None => path.join(format!("{}.md", key)),
        }
    }

    pub fn read(&self, key: &str) -> Result<Option<String>, MemoryError> {
        self.read_ns("", key)
    }

    pub fn write(&self, key: &str, value: &str) -> Result<(), MemoryError> {
        self.write_ns("", key, value)
    }

    fn read_ns(&self, namespace: &str, key: &str) -> Result<Option<String>, MemoryError> {
        let path = self.key_path(namespace, key);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| MemoryError::Io { path, source: e })?;
        let (content, meta) = parse_frontmatter(&raw);
        if meta.is_expired() {
            // Silently skip expired entries; optionally could delete the file here
            return Ok(None);
        }
        Ok(Some(content))
    }

    fn read_with_meta_ns(&self, namespace: &str, key: &str) -> Result<Option<(String, MemoryMeta)>, MemoryError> {
        let path = self.key_path(namespace, key);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| MemoryError::Io { path, source: e })?;
        let (content, meta) = parse_frontmatter(&raw);
        if meta.is_expired() {
            return Ok(None);
        }
        Ok(Some((content, meta)))
    }

    fn touch_ns(&self, namespace: &str, key: &str) -> Result<(), MemoryError> {
        let path = self.key_path(namespace, key);
        if !path.exists() {
            return Ok(());
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| MemoryError::Io { path: path.clone(), source: e })?;
        let (content, mut meta) = parse_frontmatter(&raw);
        meta.access_count += 1;
        meta.updated_at = Utc::now();
        let file_content = write_frontmatter(&meta, &content);
        std::fs::write(&path, file_content)
            .map_err(|e| MemoryError::Io { path, source: e })
    }

    fn list_keys_ns(&self, namespace: &str) -> Result<Vec<String>, MemoryError> {
        let dir = if namespace.is_empty() {
            self.base_dir.clone()
        } else {
            self.base_dir.join(namespace)
        };
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut keys = Vec::new();
        collect_keys_recursive(&dir, &dir, &mut keys)?;
        // Filter out expired entries
        keys.retain(|k| {
            if let Ok(path) = std::fs::read_to_string(self.key_path(namespace, k)) {
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
        let path = self.key_path(namespace, key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MemoryError::Io { path: parent.to_path_buf(), source: e })?;
        }
        let meta = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| MemoryError::Io { path: path.clone(), source: e })?;
            let (_, mut existing) = parse_frontmatter(&raw);
            existing.updated_at = Utc::now();
            existing
        } else {
            MemoryMeta::default()
        };
        let file_content = write_frontmatter(&meta, value);
        std::fs::write(&path, file_content)
            .map_err(|e| MemoryError::Io { path, source: e })
    }

    fn write_with_meta_ns(&self, namespace: &str, key: &str, value: &str, mut meta: MemoryMeta) -> Result<(), MemoryError> {
        let path = self.key_path(namespace, key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MemoryError::Io { path: parent.to_path_buf(), source: e })?;
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
        std::fs::write(&path, file_content)
            .map_err(|e| MemoryError::Io { path, source: e })
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
    for entry in std::fs::read_dir(dir).map_err(|e| MemoryError::Io { path: dir.to_path_buf(), source: e })? {
        let entry = entry.map_err(|e| MemoryError::Io { path: dir.to_path_buf(), source: e })?;
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
    pub fn write_with_meta(&self, key: &str, value: &str, meta: MemoryMeta) -> Result<(), MemoryError> {
        self.inner.write_with_meta_ns(&self.namespace, key, value, meta)
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
    /// Score = recency_weight * (1 / seconds_since_updated + 1) + 0.3 * access_count.
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
                let score = 0.7 * recency + 0.3 * (meta.access_count as f64).ln_1p();
                scored.push((score, key.clone(), value));
            }
        }

        // Always include repo_brain at the front if it exists
        let brain_pos = scored.iter().position(|(_, k, _)| k == "repo_brain");
        let brain_entry = brain_pos.map(|i| scored.remove(i));

        // Sort by score descending; reserve one slot for repo_brain if present
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let remaining_slots = if brain_entry.is_some() { n.saturating_sub(1) } else { n };
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
        assert_eq!(scoped.read_with_meta("key").unwrap().unwrap().1.access_count, 0);
        scoped.touch("key").unwrap();
        scoped.touch("key").unwrap();
        assert_eq!(scoped.read_with_meta("key").unwrap().unwrap().1.access_count, 2);
    }

    #[test]
    fn read_legacy_file_without_frontmatter_returns_content() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("key.md");
        std::fs::write(&path, "legacy content").unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        assert_eq!(store.read("key").unwrap(), Some("legacy content".to_string()));
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
        assert_eq!(scoped.read("arch:overview").unwrap(), Some("the system design".to_string()));
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
        assert_eq!(store.read("fresh").unwrap(), Some("fresh content".to_string()));
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
        assert_eq!(parse_duration_str("90m"), Some(chrono::Duration::minutes(90)));
        assert_eq!(parse_duration_str("invalid"), None);
    }

    #[test]
    fn read_top_n_returns_at_most_n_and_prioritises_repo_brain() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");

        // Write 5 entries + repo_brain
        for i in 0..5 {
            scoped.write(&format!("entry-{i}"), &format!("value-{i}")).unwrap();
        }
        scoped.write("repo_brain", "# Architecture overview").unwrap();

        // Top-3 should include repo_brain first and 2 others
        let result = scoped.read_top_n(3).unwrap();
        assert!(result.starts_with("### repo_brain"), "repo_brain should be first");
        // Should contain at most 3 blocks
        let blocks = result.split("\n\n").filter(|s| s.starts_with("### ")).count();
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
                p.display(), base.display()
            );
        }
        // And verify the value is readable via the sanitized path
        let val = scoped.read("passwd").unwrap();
        assert_eq!(val, Some("should not escape".to_string()));
    }
}
