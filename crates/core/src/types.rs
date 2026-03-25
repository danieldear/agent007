use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

impl Default for AgentId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Opaque ref to a prompt stored in PromptStore. Never put raw prompts on the event bus.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PromptRef(pub Uuid);

impl PromptRef {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// Opaque ref to a memory value. Never put raw memory content on the event bus.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryRef(pub Uuid);

impl MemoryRef {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// Shared, thread-safe store mapping PromptRef → raw prompt text.
#[derive(Default)]
pub struct PromptStore {
    inner: HashMap<PromptRef, String>,
}

impl PromptStore {
    pub fn insert(&mut self, prompt: String) -> PromptRef {
        let r = PromptRef::new();
        self.inner.insert(r.clone(), prompt);
        r
    }

    pub fn get(&self, r: &PromptRef) -> Option<&str> {
        self.inner.get(r).map(|s| s.as_str())
    }
}

/// Convenience alias used by WorkerAgent.
pub type SharedPromptStore = Arc<Mutex<PromptStore>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_store_insert_and_retrieve() {
        let mut store = PromptStore::default();
        let r = store.insert("my prompt".to_string());
        assert_eq!(store.get(&r), Some("my prompt"));
    }

    #[test]
    fn shared_prompt_store_accessible_across_clone() {
        let store: SharedPromptStore = Arc::new(Mutex::new(PromptStore::default()));
        let r = store.lock().unwrap().insert("shared prompt".to_string());
        let store2 = Arc::clone(&store);
        assert_eq!(store2.lock().unwrap().get(&r), Some("shared prompt"));
    }
}

