use agent007_memory::store::ScopedMemoryStore;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use uuid::Uuid;

/// Sentinel value used as the index key for feedback entries with no associated skill.
const NO_SKILL_SENTINEL: &str = "__none__";

/// Thread-safe wrapper around a scoped key-value store for learning data.
pub struct LearningStore {
    /// Uses MemoryStore::scoped("learning") — all keys prefixed with "learning/"
    scoped: ScopedMemoryStore,
    /// Serializes index read-modify-write operations to prevent lost updates under concurrency.
    index_lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptVersion {
    pub version: u32,
    pub skill_name: String,
    pub prompt_text: String,
    pub avg_reward: Option<f32>,
    pub created_at: DateTime<Utc>,
}

impl LearningStore {
    /// Construct from a ScopedMemoryStore already scoped to "learning".
    pub fn new(scoped: ScopedMemoryStore) -> Self {
        Self {
            scoped,
            index_lock: Mutex::new(()),
        }
    }

    /// Persist a FeedbackEntry (serialized as JSON) under key "feedback/<entry.id>".
    // NOTE: The data write and index write are not atomic. If a panic occurs
    // between the two, the entry will exist in storage but not in the index
    // (orphaned record). A future repair path would scan all "feedback/<uuid>"
    // keys and rebuild the index if it detects inconsistencies.
    pub fn record_feedback(
        &self,
        entry: &crate::types::FeedbackEntry,
    ) -> Result<(), crate::error::LearningError> {
        let key = format!("feedback/{}", entry.id);
        let value = serde_json::to_string(entry)?;
        self.scoped.write(&key, &value)?;

        // Update the per-skill index so we can list by skill later.
        // Index key: "feedback/index/<skill_name>" -> JSON array of uuid strings
        let skill = entry.skill_name.as_deref().unwrap_or(NO_SKILL_SENTINEL);
        let index_key = format!("feedback/index/{}", skill);
        let _guard = self.index_lock.lock().unwrap();
        let mut ids: Vec<String> = self
            .scoped
            .read(&index_key)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        ids.push(entry.id.to_string());
        let index_json = serde_json::to_string(&ids)?;
        self.scoped.write(&index_key, &index_json)?;

        Ok(())
    }

    /// Retrieve a FeedbackEntry by id.
    pub fn get_entry(
        &self,
        id: Uuid,
    ) -> Result<Option<crate::types::FeedbackEntry>, crate::error::LearningError> {
        let key = format!("feedback/{}", id);
        match self.scoped.read(&key)? {
            None => Ok(None),
            Some(s) => {
                let entry = serde_json::from_str(&s)?;
                Ok(Some(entry))
            }
        }
    }

    /// List the N most recent FeedbackEntry records for a given skill_name.
    pub fn list_recent_feedback(
        &self,
        skill_name: &str,
        n: usize,
    ) -> Result<Vec<crate::types::FeedbackEntry>, crate::error::LearningError> {
        let index_key = format!("feedback/index/{}", skill_name);
        let ids: Vec<String> = self
            .scoped
            .read(&index_key)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        // Take the last `n` ids (most recently appended).
        let recent_ids: Vec<&String> = ids.iter().rev().take(n).collect();
        let mut entries = Vec::with_capacity(recent_ids.len());
        for id_str in recent_ids.into_iter().rev() {
            if let Ok(id) = Uuid::parse_str(id_str) {
                if let Some(entry) = self.get_entry(id)? {
                    entries.push(entry);
                }
            }
        }
        Ok(entries)
    }

    /// Save an improved prompt as a new version for the given skill.
    // NOTE: The data write and index write are not atomic. If a panic occurs
    // between the two, the entry will exist in storage but not in the index
    // (orphaned record). A future repair path would scan all "versions/<skill>/<version>"
    // keys and rebuild the index if it detects inconsistencies.
    pub fn save_prompt_version(
        &self,
        version: PromptVersion,
    ) -> Result<(), crate::error::LearningError> {
        let key = format!("versions/{}/{}", version.skill_name, version.version);
        let value = serde_json::to_string(&version)?;
        self.scoped.write(&key, &value)?;

        // Update version-number index: "versions/<skill>/index" -> JSON array of u32
        let index_key = format!("versions/{}/index", version.skill_name);
        let _guard = self.index_lock.lock().unwrap();
        let mut versions: Vec<u32> = self
            .scoped
            .read(&index_key)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if !versions.contains(&version.version) {
            versions.push(version.version);
            versions.sort_unstable();
        }
        let index_json = serde_json::to_string(&versions)?;
        self.scoped.write(&index_key, &index_json)?;

        Ok(())
    }

    /// Return all saved PromptVersion records for a skill, ordered by version ascending.
    pub fn get_prompt_versions(
        &self,
        skill_name: &str,
    ) -> Result<Vec<PromptVersion>, crate::error::LearningError> {
        let index_key = format!("versions/{}/index", skill_name);
        let versions: Vec<u32> = self
            .scoped
            .read(&index_key)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let mut result = Vec::with_capacity(versions.len());
        for v in versions {
            let key = format!("versions/{}/{}", skill_name, v);
            if let Some(s) = self.scoped.read(&key)? {
                let pv: PromptVersion = serde_json::from_str(&s)?;
                result.push(pv);
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::types::{AgentId, PromptRef};
    use agent007_memory::store::MemoryStore;
    use crate::types::{FeedbackEntry, Outcome};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_store() -> (LearningStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let ms = Arc::new(MemoryStore::new(dir.path()));
        let scoped = ms.scoped("learning");
        (LearningStore::new(scoped), dir)
    }

    fn make_entry(skill: Option<&str>) -> FeedbackEntry {
        FeedbackEntry {
            id: Uuid::new_v4(),
            agent_id: AgentId::new(),
            prompt_ref: PromptRef::new(),
            skill_name: skill.map(str::to_string),
            model: "claude".to_string(),
            outcome: Outcome::Success,
            reward: Some(0.9),
            timestamp: Utc::now(),
        }
    }

    /// LearningStore::new() creates a scoped store under "learning" namespace
    #[test]
    fn new_creates_scoped_store() {
        let dir = TempDir::new().unwrap();
        let ms = Arc::new(MemoryStore::new(dir.path()));
        let scoped = ms.scoped("learning");
        assert_eq!(scoped.namespace, "learning");
        let _store = LearningStore::new(scoped);
    }

    /// record_feedback() persists a FeedbackEntry and get_entry() retrieves it by id
    #[test]
    fn record_feedback_and_get_entry() {
        let (store, _dir) = make_store();
        let entry = make_entry(Some("code-review"));

        store.record_feedback(&entry).unwrap();

        let retrieved = store.get_entry(entry.id).unwrap().expect("should find entry");
        assert_eq!(retrieved.id, entry.id);
        assert_eq!(retrieved.model, "claude");
        assert_eq!(retrieved.skill_name, Some("code-review".to_string()));
    }

    /// list_recent_feedback(skill_name, n) returns at most n entries for the given skill
    #[test]
    fn list_recent_feedback_returns_at_most_n() {
        let (store, _dir) = make_store();

        // Record 5 entries for "review-pr" and 2 for "other"
        for _ in 0..5 {
            store.record_feedback(&make_entry(Some("review-pr"))).unwrap();
        }
        for _ in 0..2 {
            store.record_feedback(&make_entry(Some("other"))).unwrap();
        }

        // Ask for at most 3
        let results = store.list_recent_feedback("review-pr", 3).unwrap();
        assert_eq!(results.len(), 3);
        // All should be for "review-pr"
        for e in &results {
            assert_eq!(e.skill_name, Some("review-pr".to_string()));
        }

        // "other" skill untouched
        let other = store.list_recent_feedback("other", 10).unwrap();
        assert_eq!(other.len(), 2);
    }

    /// save_prompt_version() stores a new version; get_prompt_versions() returns all versions in order
    #[test]
    fn save_and_get_prompt_versions() {
        let (store, _dir) = make_store();

        let v1 = PromptVersion {
            version: 1,
            skill_name: "summarize".to_string(),
            prompt_text: "Summarize this:".to_string(),
            avg_reward: Some(0.5),
            created_at: Utc::now(),
        };
        let v2 = PromptVersion {
            version: 2,
            skill_name: "summarize".to_string(),
            prompt_text: "Please summarize:".to_string(),
            avg_reward: Some(0.8),
            created_at: Utc::now(),
        };

        store.save_prompt_version(v2).unwrap(); // intentionally save out of order
        store.save_prompt_version(v1).unwrap();

        let versions = store.get_prompt_versions("summarize").unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 1, "should be ordered ascending");
        assert_eq!(versions[1].version, 2);
        assert_eq!(versions[0].prompt_text, "Summarize this:");
        assert_eq!(versions[1].prompt_text, "Please summarize:");
    }

    /// get_prompt_versions() returns empty list for unknown skill
    #[test]
    fn get_prompt_versions_unknown_skill_returns_empty() {
        let (store, _dir) = make_store();
        let versions = store.get_prompt_versions("nonexistent-skill").unwrap();
        assert!(versions.is_empty());
    }
}
