// crates/skills/src/provider.rs
use crate::types::Skill;
use std::collections::HashMap;

/// A read-only view into loaded skills, keyed by normalised trigger name.
///
/// Separate from [`crate::loader::SkillLoader`] (which handles filesystem I/O)
/// to allow mocking in tests and avoid re-reading the filesystem per lookup.
pub trait SkillContentProvider: Send + Sync {
    /// Returns the Markdown body (template field) for the given trigger name.
    ///
    /// Trigger is normalised before lookup: a leading `/` is stripped so that
    /// both `"dev-debug"` and `"/dev-debug"` resolve to the same skill.
    ///
    /// Returns `None` if no skill matches. Callers MUST NOT panic on `None`;
    /// they should silently skip injection.
    fn load_content(&self, trigger: &str) -> Option<String>;
}

/// Normalise a trigger string: strip leading `'/'` for key comparison.
pub fn normalize_trigger(t: &str) -> &str {
    t.strip_prefix('/').unwrap_or(t)
}

/// In-memory index of loaded skills, keyed by normalised trigger.
///
/// Build once from a `Vec<Skill>` (returned by `SkillLoader::load_all()`),
/// then share behind an `Arc<dyn SkillContentProvider>`.
pub struct SkillIndex {
    by_trigger: HashMap<String, String>, // normalised_trigger → template body
}

impl SkillIndex {
    /// Build an index from a collection of loaded skills.
    /// If multiple skills have the same normalised trigger, the last one wins
    /// (callers should deduplicate before calling this if ordering matters).
    pub fn from_skills(skills: Vec<Skill>) -> Self {
        let mut by_trigger = HashMap::new();
        for skill in skills {
            let key = normalize_trigger(skill.trigger()).to_string();
            by_trigger.insert(key, skill.template.clone());
        }
        Self { by_trigger }
    }

    /// Number of skills in the index.
    pub fn len(&self) -> usize {
        self.by_trigger.len()
    }

    /// `true` if the index contains no skills.
    pub fn is_empty(&self) -> bool {
        self.by_trigger.is_empty()
    }
}

impl SkillContentProvider for SkillIndex {
    fn load_content(&self, trigger: &str) -> Option<String> {
        let key = normalize_trigger(trigger);
        self.by_trigger.get(key).cloned()
    }
}

/// A no-op provider that always returns `None`. Useful as a fallback when no
/// skill index has been wired up (e.g. in tests or CLI modes that skip skills).
pub struct NoOpSkillContentProvider;

impl SkillContentProvider for NoOpSkillContentProvider {
    fn load_content(&self, _trigger: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Skill, SkillFrontmatter};
    use std::path::PathBuf;

    fn make_skill(trigger: &str, template: &str) -> Skill {
        Skill {
            frontmatter: SkillFrontmatter {
                name: trigger.to_string(),
                description: "test skill".to_string(),
                trigger: trigger.to_string(),
                model: "claude".to_string(),
                category: "test".to_string(),
                version: "1.0.0".to_string(),
                tags: vec![],
            },
            template: template.to_string(),
            manifest_path: PathBuf::from("test.md"),
            entry_path: PathBuf::from("test.md"),
            skill_dir: PathBuf::from("."),
        }
    }

    #[test]
    fn load_content_by_exact_trigger() {
        let skills = vec![make_skill("/dev-debug", "debug domain knowledge")];
        let index = SkillIndex::from_skills(skills);
        assert_eq!(
            index.load_content("/dev-debug"),
            Some("debug domain knowledge".to_string())
        );
    }

    #[test]
    fn load_content_normalizes_slash_prefix() {
        let skills = vec![make_skill("/dev-debug", "debug body")];
        let index = SkillIndex::from_skills(skills);
        // caller passes without slash — must still resolve
        assert_eq!(
            index.load_content("dev-debug"),
            Some("debug body".to_string())
        );
    }

    #[test]
    fn load_content_trigger_stored_without_slash() {
        // If the skill trigger itself has no slash, lookup with slash must work too
        let skills = vec![make_skill("wifi-debug", "wifi knowledge")];
        let index = SkillIndex::from_skills(skills);
        assert_eq!(
            index.load_content("/wifi-debug"),
            Some("wifi knowledge".to_string())
        );
        assert_eq!(
            index.load_content("wifi-debug"),
            Some("wifi knowledge".to_string())
        );
    }

    #[test]
    fn load_content_unknown_trigger_returns_none() {
        let index = SkillIndex::from_skills(vec![]);
        assert!(index.load_content("nonexistent").is_none());
    }

    #[test]
    fn noop_provider_always_returns_none() {
        let p = NoOpSkillContentProvider;
        assert!(p.load_content("anything").is_none());
        assert!(p.load_content("/dev-debug").is_none());
    }

    #[test]
    fn index_len_reflects_skill_count() {
        let skills = vec![
            make_skill("/a", "body a"),
            make_skill("/b", "body b"),
        ];
        let index = SkillIndex::from_skills(skills);
        assert_eq!(index.len(), 2);
        assert!(!index.is_empty());
    }

    #[test]
    fn empty_index_is_empty() {
        let index = SkillIndex::from_skills(vec![]);
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn normalize_trigger_strips_slash() {
        assert_eq!(normalize_trigger("/foo"), "foo");
        assert_eq!(normalize_trigger("foo"), "foo");
        assert_eq!(normalize_trigger("//foo"), "/foo"); // only strips one
    }
}
