// The `agent` command operates on deprecated `AgentDef` TOML files, which stay
// supported for backward compatibility alongside the newer `PersonaSpec` path.
#![allow(deprecated)]

use agent007_core::dispatcher::LocalDispatcher;
use agent007_core::persona::{PersonaProvider, PersonaSpec};
use agent007_custom_agents::{
    AgentDef, AgentRegistry, AgentType, CustomAgentError, SubOrchestrator,
};
use agent007_memory::store::MemoryStore;
use agent007_models::router::ModelRouter;
use std::sync::Arc;

/// Render a table-style listing of all registered agents.
pub fn format_list(registry: &AgentRegistry) -> String {
    let mut lines = vec![
        format!("{:<30} {:<18} {}", "NAME", "TYPE", "DESCRIPTION"),
        "-".repeat(70),
    ];
    let mut agents = registry.list();
    agents.sort_by_key(|a| &a.name);
    for def in agents {
        let type_str = match def.r#type {
            AgentType::Worker => "worker",
            AgentType::SubOrchestrator => "sub-orchestrator",
        };
        let desc = def.description.as_deref().unwrap_or("-");
        lines.push(format!("{:<30} {:<18} {}", def.name, type_str, desc));
    }
    lines.join("\n")
}

/// Render detailed info for a single agent.
pub fn format_inspect(def: &AgentDef) -> String {
    let mut lines = vec![
        format!("Name:             {}", def.name),
        format!("Type:             {:?}", def.r#type),
        format!(
            "Description:      {}",
            def.description.as_deref().unwrap_or("-")
        ),
        format!(
            "Memory Namespace: {}",
            def.memory_namespace.as_deref().unwrap_or(&def.name)
        ),
        format!(
            "Model:            {}",
            def.model.as_deref().unwrap_or("default")
        ),
    ];
    if let Some(ref scope) = def.scope {
        lines.push(format!("Scope:            {}", scope.join(", ")));
    }
    if let Some(ref workers) = def.allowed_workers {
        lines.push(format!("Allowed Workers:  {}", workers.join(", ")));
    }
    if let Some(ref zones) = def.zones {
        if let Some(ref ro) = zones.readonly {
            lines.push(format!("Zones (readonly): {}", ro.join(", ")));
        }
        if let Some(ref sens) = zones.sensitive {
            lines.push(format!("Zones (sensitive): {}", sens.join(", ")));
        }
        if let Some(ref forb) = zones.forbidden {
            lines.push(format!("Zones (forbidden): {}", forb.join(", ")));
        }
    }
    lines.join("\n")
}

/// Look up an agent by name and return formatted inspect output.
pub fn inspect_agent(registry: &AgentRegistry, name: &str) -> Result<String, CustomAgentError> {
    let def = registry
        .get(name)
        .ok_or_else(|| CustomAgentError::NotFound {
            name: name.to_string(),
        })?;
    Ok(format_inspect(def))
}

/// Generate a minimal TOML stub for a new agent.
pub fn generate_agent_toml(name: &str, agent_type: &str, namespace: Option<&str>) -> String {
    let ns_line = namespace
        .map(|ns| format!("memory_namespace = \"{ns}\"\n"))
        .unwrap_or_default();
    format!(
        r#"name = "{name}"
type = "{agent_type}"
description = "TODO: describe this agent"
system_prompt = "TODO: write the system prompt for {name}."
{ns_line}allowed_workers = []
"#
    )
}

/// Sub-actions for the `agent` command.
#[derive(Debug, Clone)]
pub enum AgentAction {
    List,
    Inspect {
        name: String,
    },
    Run {
        name: String,
        task: String,
    },
    Create {
        name: String,
        agent_type: String,
        namespace: Option<String>,
    },
}

/// Entry point called from main.rs dispatch.
///
/// Accepts pre-built stack dependencies so `agent run` can reuse the same
/// model router, dispatcher, memory store, and persona registry that the
/// full `run` command uses.
pub async fn execute(
    registry: Arc<AgentRegistry>,
    action: AgentAction,
    model_router: Arc<ModelRouter>,
    dispatcher: Arc<LocalDispatcher>,
    memory_store: Arc<MemoryStore>,
    persona_registry: Arc<dyn PersonaProvider>,
) -> anyhow::Result<()> {
    match action {
        AgentAction::List => {
            println!("{}", format_list(&registry));
        }
        AgentAction::Inspect { name } => {
            println!("{}", inspect_agent(&registry, &name)?);
        }
        AgentAction::Run { name, task } => {
            let orch = if let Some(persona) = persona_registry.get(&name) {
                build_persona_orchestrator(
                    persona,
                    Arc::clone(&memory_store),
                    model_router,
                    persona_registry,
                    dispatcher as Arc<dyn agent007_core::dispatcher::Dispatcher>,
                )?
            } else {
                let def = registry
                    .get(&name)
                    .ok_or_else(|| CustomAgentError::NotFound { name: name.clone() })?
                    .clone();

                let ns = def
                    .memory_namespace
                    .clone()
                    .unwrap_or_else(|| def.name.clone());
                let scoped = Arc::new(memory_store.scoped(&ns));

                SubOrchestrator::new(
                    def,
                    scoped,
                    model_router,
                    persona_registry,
                    dispatcher as Arc<dyn agent007_core::dispatcher::Dispatcher>,
                    0,
                    3,
                )
            };

            println!("🤖 Running agent '{name}' …\n   Task: {task}\n");
            match orch.run(&task).await {
                Ok(result) => {
                    println!("{}", result.output);
                    if !result.blockers.is_empty() {
                        println!("\n⚠️  Blockers:");
                        for b in &result.blockers {
                            println!("  • {b}");
                        }
                    }
                    if !result.files_changed.is_empty() {
                        println!("\n📂 Files changed:");
                        for f in &result.files_changed {
                            println!("  • {}", f.display());
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Agent run failed: {e}");
                    return Err(anyhow::anyhow!("{e}"));
                }
            }
        }
        AgentAction::Create {
            name,
            agent_type,
            namespace,
        } => {
            let toml_str = generate_agent_toml(&name, &agent_type, namespace.as_deref());
            let home = std::env::var("AGENT007_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    std::env::var("HOME")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|_| std::path::PathBuf::from("."))
                        .join(".agent007")
                });
            let agents_dir = home.join("agents");
            std::fs::create_dir_all(&agents_dir)?;
            let path = agents_dir.join(format!("{}.toml", name.to_lowercase().replace(' ', "_")));
            std::fs::write(&path, &toml_str)?;
            println!("Created agent definition at: {}", path.display());
            println!("\n{toml_str}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const WORKER_TOML: &str = r#"
        name = "WorkerA"
        type = "worker"
        system_prompt = "I work hard."
        description = "A simple worker"
    "#;

    const ORCH_TOML: &str = r#"
        name = "OrchestratorB"
        type = "sub-orchestrator"
        system_prompt = "I orchestrate."
        allowed_workers = ["WorkerA"]
        memory_namespace = "orch-ns"
        scope = ["src/foo/"]
    "#;

    fn setup_agents_dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().unwrap();
        let agents_dir = dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(agents_dir.join("worker.toml"), WORKER_TOML).unwrap();
        fs::write(agents_dir.join("orch.toml"), ORCH_TOML).unwrap();
        (dir, agents_dir)
    }

    #[test]
    fn list_output_contains_agent_names() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let output = format_list(&registry);
        assert!(output.contains("WorkerA"));
        assert!(output.contains("OrchestratorB"));
    }

    #[test]
    fn list_output_shows_agent_type() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let output = format_list(&registry);
        assert!(output.contains("worker") || output.contains("Worker"));
        assert!(output.contains("sub-orchestrator") || output.contains("SubOrchestrator"));
    }

    #[test]
    fn inspect_output_contains_scope_and_namespace() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let def = registry.get("OrchestratorB").unwrap();
        let output = format_inspect(def);
        assert!(output.contains("src/foo/"));
        assert!(output.contains("orch-ns"));
    }

    #[test]
    fn inspect_output_contains_allowed_workers() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let def = registry.get("OrchestratorB").unwrap();
        let output = format_inspect(def);
        assert!(output.contains("WorkerA"));
    }

    #[test]
    fn inspect_unknown_agent_returns_error() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let err = inspect_agent(&registry, "NonExistent").unwrap_err();
        assert!(matches!(
            err,
            agent007_custom_agents::CustomAgentError::NotFound { .. }
        ));
    }

    #[test]
    fn create_wizard_generates_valid_toml() {
        let toml_str = generate_agent_toml("NewAgent", "sub-orchestrator", Some("new-ns"));
        let def: agent007_custom_agents::AgentDef = toml::from_str(&toml_str).unwrap();
        assert_eq!(def.name, "NewAgent");
        assert_eq!(def.memory_namespace.as_deref(), Some("new-ns"));
    }
}

fn build_persona_orchestrator(
    persona: PersonaSpec,
    memory_store: Arc<MemoryStore>,
    model_router: Arc<ModelRouter>,
    persona_registry: Arc<dyn PersonaProvider>,
    dispatcher: Arc<dyn agent007_core::dispatcher::Dispatcher>,
) -> anyhow::Result<SubOrchestrator> {
    if !matches!(
        persona.agent_type.as_deref(),
        Some(kind) if kind.eq_ignore_ascii_case("orchestrator")
    ) {
        return Err(CustomAgentError::InvalidPersonaType {
            name: persona.name.clone(),
            expected: "orchestrator".to_string(),
        }
        .into());
    }
    let ns = persona
        .memory_namespace
        .clone()
        .unwrap_or_else(|| persona.name.clone());
    let scoped = Arc::new(memory_store.scoped(&ns));
    let skill_provider: Arc<dyn agent007_skills::SkillContentProvider> =
        match agent007_skills::SkillLoader::load_from_dirs(
            agent007_core::paths::skills_search_dirs(),
        ) {
            Ok(skills) => Arc::new(agent007_skills::SkillIndex::from_skills(skills)),
            Err(_) => Arc::new(agent007_skills::NoOpSkillContentProvider),
        };

    Ok(SubOrchestrator::from_persona(
        &persona,
        Vec::new(),
        skill_provider,
        scoped,
        model_router,
        persona_registry,
        dispatcher,
        0,
        3,
    ))
}
