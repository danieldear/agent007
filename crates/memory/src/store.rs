use std::path::PathBuf;
use std::sync::Arc;
use crate::error::MemoryError;

pub struct MemoryStore {
    base_dir: PathBuf,
}

impl MemoryStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }

    fn key_path(&self, namespace: &str, key: &str) -> PathBuf {
        if namespace.is_empty() {
            self.base_dir.join(format!("{}.md", key))
        } else {
            self.base_dir.join(namespace).join(format!("{}.md", key))
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
        std::fs::read_to_string(&path)
            .map(Some)
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
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| MemoryError::Io { path: dir.clone(), source: e })?
        {
            let entry = entry.map_err(|e| MemoryError::Io { path: dir.clone(), source: e })?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    keys.push(stem.to_string());
                }
            }
        }
        keys.sort();
        Ok(keys)
    }

    fn write_ns(&self, namespace: &str, key: &str, value: &str) -> Result<(), MemoryError> {
        let path = self.key_path(namespace, key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MemoryError::Io { path: parent.to_path_buf(), source: e })?;
        }
        std::fs::write(&path, value)
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

    /// List all keys stored in this scope.
    pub fn list_keys(&self) -> Result<Vec<String>, MemoryError> {
        self.inner.list_keys_ns(&self.namespace)
    }

    /// Read all key→value pairs in this scope, concatenated as "key: value\n\n" blocks.
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
        // global read should NOT see the scoped write
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
}
