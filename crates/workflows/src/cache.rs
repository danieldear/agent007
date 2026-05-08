use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Content-addressed step output cache.
///
/// Cache key = SHA256(step_id + rendered_prompt + all referenced file mtimes).
/// Entries stored as `.agent007/runtime/step_cache/<hex>.json`.
pub struct StepCache {
    cache_dir: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct CacheEntry {
    pub key: String,
    pub step_id: String,
    pub output: String,
    pub cached_at: String,
}

impl StepCache {
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            cache_dir: workspace_root.join(".agent007/runtime/step_cache"),
        }
    }

    /// Compute cache key for a step.
    pub fn compute_key(step_id: &str, rendered_prompt: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(step_id.as_bytes());
        hasher.update(b"\x00");
        hasher.update(rendered_prompt.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }

    /// Compute cache key incorporating input file mtimes for cache invalidation.
    pub fn compute_key_with_files(step_id: &str, rendered_prompt: &str, paths: &[&str]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(step_id.as_bytes());
        hasher.update(b"\x00");
        hasher.update(rendered_prompt.as_bytes());
        for path in paths {
            hasher.update(b"\x00");
            hasher.update(path.as_bytes());
            // Include mtime so cache invalidates when file changes
            if let Ok(meta) = std::fs::metadata(path) {
                if let Ok(mtime) = meta.modified() {
                    let ts = mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    hasher.update(ts.to_le_bytes());
                }
            }
        }
        let result = hasher.finalize();
        hex::encode(result)
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join(format!("{key}.json"))
    }

    /// Look up a cache entry. Returns `Some(output)` on hit, `None` on miss.
    pub fn get(&self, key: &str) -> Option<String> {
        let path = self.entry_path(key);
        let data = std::fs::read_to_string(&path).ok()?;
        let entry: CacheEntry = serde_json::from_str(&data).ok()?;
        Some(entry.output)
    }

    /// Store an output in the cache.
    pub fn put(&self, key: &str, step_id: &str, output: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.cache_dir)?;
        let entry = CacheEntry {
            key: key.to_string(),
            step_id: step_id.to_string(),
            output: output.to_string(),
            cached_at: chrono::Utc::now().to_rfc3339(),
        };
        let json = serde_json::to_string_pretty(&entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(self.entry_path(key), json)?;
        Ok(())
    }

    /// Clear all cache entries. Called by `agent007 cache clear`.
    pub fn clear(&self) -> std::io::Result<usize> {
        if !self.cache_dir.exists() {
            return Ok(0);
        }
        let mut count = 0usize;
        for entry in std::fs::read_dir(&self.cache_dir)?.flatten() {
            if entry
                .path()
                .extension()
                .map(|e| e == "json")
                .unwrap_or(false)
            {
                std::fs::remove_file(entry.path())?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Return stats: number of entries + total size in bytes.
    pub fn stats(&self) -> (usize, u64) {
        if !self.cache_dir.exists() {
            return (0, 0);
        }
        let mut count = 0usize;
        let mut total_bytes = 0u64;
        if let Ok(rd) = std::fs::read_dir(&self.cache_dir) {
            for entry in rd.flatten() {
                if entry
                    .path()
                    .extension()
                    .map(|e| e == "json")
                    .unwrap_or(false)
                {
                    count += 1;
                    total_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
        (count, total_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_cache() -> (TempDir, StepCache) {
        let tmp = tempfile::tempdir().unwrap();
        let cache = StepCache::new(tmp.path());
        (tmp, cache)
    }

    #[test]
    fn miss_returns_none() {
        let (_tmp, cache) = tmp_cache();
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn put_then_get_roundtrip() {
        let (_tmp, cache) = tmp_cache();
        let key = StepCache::compute_key("step-1", "my prompt");
        cache.put(&key, "step-1", "step output").unwrap();
        let hit = cache.get(&key).unwrap();
        assert_eq!(hit, "step output");
    }

    #[test]
    fn different_prompts_different_keys() {
        let k1 = StepCache::compute_key("step-1", "prompt A");
        let k2 = StepCache::compute_key("step-1", "prompt B");
        assert_ne!(k1, k2);
    }

    #[test]
    fn clear_removes_entries() {
        let (_tmp, cache) = tmp_cache();
        let key = StepCache::compute_key("step-1", "prompt");
        cache.put(&key, "step-1", "output").unwrap();
        let removed = cache.clear().unwrap();
        assert_eq!(removed, 1);
        assert!(cache.get(&key).is_none());
    }
}
