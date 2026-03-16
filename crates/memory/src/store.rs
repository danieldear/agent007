use std::path::{Path, PathBuf};
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
}
