// crates/core/src/persona.rs
use serde::{Deserialize, Serialize};

/// Full specification for a single persona.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaSpec {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub preferred_model: String,
    pub allowed_tools: Vec<String>,
}

/// Trait for any store of personas.  Must be Send + Sync so it can be held in Arc<dyn PersonaProvider>.
pub trait PersonaProvider: Send + Sync {
    /// Look up a persona by exact name (case-sensitive).
    fn get(&self, name: &str) -> Option<PersonaSpec>;

    /// List all available personas.
    fn list(&self) -> Vec<PersonaSpec>;
}

/// Stub implementation — always returns None / empty.  Used before PersonaRegistry is wired in.
pub struct NoOpPersonaProvider;

impl PersonaProvider for NoOpPersonaProvider {
    fn get(&self, _name: &str) -> Option<PersonaSpec> {
        None
    }

    fn list(&self) -> Vec<PersonaSpec> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_provider_returns_none_and_empty() {
        let p = NoOpPersonaProvider;
        assert!(p.get("Researcher").is_none());
        assert!(p.list().is_empty());
    }

    #[test]
    fn persona_spec_fields_are_accessible() {
        let spec = PersonaSpec {
            name: "Test".to_string(),
            description: "desc".to_string(),
            system_prompt: "you are...".to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec!["bash".to_string()],
        };
        assert_eq!(spec.name, "Test");
        assert_eq!(spec.preferred_model, "claude");
        assert_eq!(spec.allowed_tools.len(), 1);
    }
}
