use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentType {
    Worker,
    SubOrchestrator,
}

/// Minimal per-worker configuration used by `SubOrchestrator::from_persona`.
///
/// Callers that build workers from a `WorkerConfig` (workflows crate) convert
/// to this type to avoid a circular dependency between `custom-agents` and
/// `workflows`.
#[derive(Debug, Clone, Default)]
pub struct WorkerSpec {
    /// The persona name of this worker.
    pub name: String,
    /// Skill trigger names injected into this worker's system prompt for the
    /// current invocation. These are merged with the worker persona's own
    /// default `skills` list.
    pub skills: Vec<String>,
    /// If `true`, this worker runs *after* all non-sequential workers complete
    /// and receives their combined outputs as context. Corresponds to
    /// `run = "sequential"` in the workflow TOML.
    pub sequential: bool,
}

/// Legacy agent definition loaded from `~/.agent007/agents/*.toml`.
///
/// Prefer defining agents as persona TOMLs with `agent_type = "orchestrator"` or
/// `agent_type = "worker"`. `AgentDef` files continue to work for backward
/// compatibility but are no longer required.
#[deprecated(
    note = "Use PersonaSpec with agent_type = \"worker\" | \"orchestrator\" instead. \
            AgentDef TOML files remain supported for backward compat."
)]
#[derive(Deserialize, Debug, Clone)]
pub struct AgentDef {
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: AgentType,
    pub description: Option<String>,
    pub scope: Option<Vec<String>>,
    pub system_prompt: String,
    pub allowed_workers: Option<Vec<String>>,
    pub model: Option<String>,
    pub memory_namespace: Option<String>,
    pub zones: Option<AgentZoneOverrides>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AgentZoneOverrides {
    pub readonly: Option<Vec<String>>,
    pub sensitive: Option<Vec<String>>,
    pub forbidden: Option<Vec<String>>,
}

#[derive(Debug, Default)]
pub struct SubTaskResult {
    pub output: String,
    pub files_changed: Vec<PathBuf>,
    pub tests_passed: bool,
    pub blockers: Vec<String>,
    pub token_estimate: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_type_deserializes_worker() {
        #[derive(serde::Deserialize)]
        struct W {
            t: AgentType,
        }
        let v: W = toml::from_str("t = \"worker\"").unwrap();
        assert_eq!(v.t, AgentType::Worker);
    }

    #[test]
    fn agent_type_deserializes_sub_orchestrator() {
        #[derive(serde::Deserialize)]
        struct W {
            t: AgentType,
        }
        let v: W = toml::from_str("t = \"sub-orchestrator\"").unwrap();
        assert_eq!(v.t, AgentType::SubOrchestrator);
    }

    #[test]
    fn agent_def_full_round_trip() {
        let toml_str = r#"
            name = "LibP2PSubOrchestrator"
            type = "sub-orchestrator"
            description = "Owns the libp2p networking module"
            scope = ["src/networking/libp2p/", "tests/networking/libp2p/"]
            system_prompt = "You are the LibP2P module owner."
            allowed_workers = ["Researcher", "Coder"]
            model = "claude"
            memory_namespace = "libp2p"

            [zones]
            readonly = ["src/networking/libp2p/core/"]
        "#;
        let def: AgentDef = toml::from_str(toml_str).unwrap();
        assert_eq!(def.name, "LibP2PSubOrchestrator");
        assert_eq!(def.r#type, AgentType::SubOrchestrator);
        assert_eq!(def.memory_namespace.as_deref(), Some("libp2p"));
        let zones = def.zones.unwrap();
        assert_eq!(zones.readonly.unwrap(), vec!["src/networking/libp2p/core/"]);
    }

    #[test]
    fn agent_def_minimal_round_trip() {
        let toml_str = r#"
            name = "QuickWorker"
            type = "worker"
            system_prompt = "Do things."
        "#;
        let def: AgentDef = toml::from_str(toml_str).unwrap();
        assert_eq!(def.r#type, AgentType::Worker);
        assert!(def.description.is_none());
        assert!(def.zones.is_none());
    }

    #[test]
    fn sub_task_result_defaults() {
        let r = SubTaskResult::default();
        assert!(r.output.is_empty());
        assert!(r.files_changed.is_empty());
        assert!(!r.tests_passed);
        assert!(r.blockers.is_empty());
    }
}
