use crate::loader::load_all;
use crate::{AgentDef, AgentType, CustomAgentError};
use std::collections::HashMap;
use std::path::Path;

pub struct AgentRegistry {
    agents: HashMap<String, AgentDef>,
}

impl AgentRegistry {
    pub fn load(agents_dir: &Path) -> Result<Self, CustomAgentError> {
        let defs = load_all(agents_dir)?;
        let agents = defs.into_iter().map(|d| (d.name.clone(), d)).collect();
        Ok(Self { agents })
    }

    pub fn empty() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&AgentDef> {
        self.agents.get(name)
    }

    pub fn list(&self) -> Vec<&AgentDef> {
        self.agents.values().collect()
    }

    pub fn sub_orchestrators(&self) -> Vec<&AgentDef> {
        self.agents
            .values()
            .filter(|d| d.r#type == AgentType::SubOrchestrator)
            .collect()
    }

    pub fn workers(&self) -> Vec<&AgentDef> {
        self.agents
            .values()
            .filter(|d| d.r#type == AgentType::Worker)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const WORKER_TOML: &str = r#"
        name = "WorkerA"
        type = "worker"
        system_prompt = "I work."
    "#;

    const ORCHESTRATOR_TOML: &str = r#"
        name = "OrchestratorB"
        type = "sub-orchestrator"
        system_prompt = "I orchestrate."
        allowed_workers = ["WorkerA"]
    "#;

    fn make_registry() -> (AgentRegistry, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("worker.toml"), WORKER_TOML).unwrap();
        fs::write(dir.path().join("orch.toml"), ORCHESTRATOR_TOML).unwrap();
        let reg = AgentRegistry::load(dir.path()).unwrap();
        (reg, dir)
    }

    #[test]
    fn load_populates_registry() {
        let (reg, _dir) = make_registry();
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn get_returns_existing_agent() {
        let (reg, _dir) = make_registry();
        let def = reg.get("WorkerA").unwrap();
        assert_eq!(def.name, "WorkerA");
    }

    #[test]
    fn get_returns_none_for_unknown() {
        let (reg, _dir) = make_registry();
        assert!(reg.get("Nobody").is_none());
    }

    #[test]
    fn workers_filter_returns_only_workers() {
        let (reg, _dir) = make_registry();
        let workers = reg.workers();
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].name, "WorkerA");
    }

    #[test]
    fn sub_orchestrators_filter_returns_only_orchestrators() {
        let (reg, _dir) = make_registry();
        let orchs = reg.sub_orchestrators();
        assert_eq!(orchs.len(), 1);
        assert_eq!(orchs[0].name, "OrchestratorB");
    }

    #[test]
    fn load_missing_dir_returns_empty_registry() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nonexistent");
        let reg = AgentRegistry::load(&missing).unwrap();
        assert!(reg.list().is_empty());
    }
}
