// crates/core/src/persona.rs
use agent007_zones::ZoneConfig;
use serde::{Deserialize, Serialize};

/// Full specification for a single persona.
///
/// A persona IS the agent definition — no separate AgentDef TOML required.
/// All new fields are optional with `#[serde(default)]` for backward compat:
/// existing persona TOML files without these fields continue to load unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaSpec {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub preferred_model: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    // ── Agent lifecycle fields (new — all optional) ───────────────────────────
    /// Scoped memory key prefix. Defaults to `self.name` at runtime when `None`.
    #[serde(default)]
    pub memory_namespace: Option<String>,

    /// File-path zone rules. Uses the same `ZoneConfig` as the rest of the system.
    #[serde(default)]
    pub zones: Option<ZoneConfig>,

    /// Default skill trigger names always loaded for this persona.
    /// E.g. `["dev-debug"]` — their Markdown bodies are prepended to the
    /// system prompt at invocation time.
    #[serde(default)]
    pub skills: Vec<String>,

    /// `"worker"` or `"orchestrator"`. Defaults to `"worker"` when `None`.
    #[serde(default)]
    pub agent_type: Option<String>,

    /// Worker persona names. Only meaningful when `agent_type = "orchestrator"`.
    #[serde(default)]
    pub allowed_workers: Option<Vec<String>>,
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
            memory_namespace: None,
            zones: None,
            skills: vec![],
            agent_type: None,
            allowed_workers: None,
        };
        assert_eq!(spec.name, "Test");
        assert_eq!(spec.preferred_model, "claude");
        assert_eq!(spec.allowed_tools.len(), 1);
    }

    #[test]
    fn persona_spec_new_fields_default_when_absent() {
        // Simulate deserialization from a minimal TOML without new fields
        let toml_str = r#"
name = "Minimal"
description = "minimal persona"
system_prompt = "You are minimal."
preferred_model = "claude"
allowed_tools = []
"#;
        let spec: PersonaSpec = toml::from_str(toml_str).unwrap();
        assert!(spec.memory_namespace.is_none());
        assert!(spec.zones.is_none());
        assert!(spec.skills.is_empty());
        assert!(spec.agent_type.is_none());
        assert!(spec.allowed_workers.is_none());
    }

    #[test]
    fn persona_spec_new_fields_roundtrip() {
        let toml_str = r#"
name = "Orchestrator"
description = "manages workers"
system_prompt = "You are an orchestrator."
preferred_model = "claude"
allowed_tools = []
memory_namespace = "orch-ns"
agent_type = "orchestrator"
skills = ["dev-debug", "code-review"]
allowed_workers = ["coder", "debugger"]

[zones]
forbidden = [".env"]
readonly = ["src/auth/"]
"#;
        let spec: PersonaSpec = toml::from_str(toml_str).unwrap();
        assert_eq!(spec.memory_namespace.as_deref(), Some("orch-ns"));
        assert_eq!(spec.agent_type.as_deref(), Some("orchestrator"));
        assert_eq!(spec.skills, vec!["dev-debug", "code-review"]);
        assert_eq!(
            spec.allowed_workers.as_deref(),
            Some(&["coder".to_string(), "debugger".to_string()][..])
        );
        let zones = spec.zones.unwrap();
        assert_eq!(zones.forbidden, vec![".env"]);
        assert_eq!(zones.readonly, vec!["src/auth/"]);
    }
}
