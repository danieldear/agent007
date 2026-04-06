use std::path::PathBuf;
use std::sync::Arc;
use chrono::{DateTime, Utc};
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
        }
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
            self.base_dir.join(namespace)
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
        let (content, _) = parse_frontmatter(&raw);
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
}
