// crates/cli/src/commands/persona.rs
use super::run::{agent007_global_home, agent007_project_home};
use crate::PersonaAction;
use agent007_core::PersonaProvider;
use agent007_personas::PersonaRegistry;
use anyhow::Result;
use std::sync::Arc;

/// Top-level dispatch for `agent007 persona <action>`.
pub async fn execute(_config: Arc<crate::config::Config>, action: PersonaAction) -> Result<()> {
    let mut dirs = Vec::new();
    if let Some(project_home) = agent007_project_home() {
        dirs.push(project_home.join("personas"));
    }
    let global_dir = agent007_global_home().join("personas");
    if !dirs.iter().any(|dir| dir == &global_dir) {
        dirs.push(global_dir);
    }
    let registry = PersonaRegistry::load_from_dirs(dirs.iter().map(|dir| dir.as_path()))
        .unwrap_or_else(|e| {
            tracing::warn!("failed to load persona overrides: {}", e);
            PersonaRegistry::built_in()
        });

    match action {
        PersonaAction::List => {
            let personas = registry.list();
            if personas.is_empty() {
                println!("No personas available.");
            } else {
                println!(
                    "{:<22} {:<14} {:<10} {}",
                    "NAME", "TYPE", "MODEL", "DESCRIPTION"
                );
                println!("{}", "-".repeat(88));
                for p in personas {
                    println!(
                        "{:<22} {:<14} {:<10} {}",
                        p.name,
                        p.agent_type.as_deref().unwrap_or("worker"),
                        p.preferred_model,
                        p.description
                    );
                }
            }
        }
        PersonaAction::Show { name } => match registry.get(&name) {
            Some(spec) => {
                println!("Name:            {}", spec.name);
                println!("Model:           {}", spec.preferred_model);
                println!("Description:     {}", spec.description);
                println!(
                    "Agent type:      {}",
                    spec.agent_type.as_deref().unwrap_or("worker")
                );
                println!(
                    "Memory ns:       {}",
                    spec.memory_namespace.as_deref().unwrap_or(&spec.name)
                );
                if let Some(workers) = &spec.allowed_workers {
                    if !workers.is_empty() {
                        println!("Allowed workers: {}", workers.join(", "));
                    }
                }
                if !spec.skills.is_empty() {
                    println!("Skills:          {}", spec.skills.join(", "));
                }
                if !spec.allowed_tools.is_empty() {
                    println!("Allowed tools:   {}", spec.allowed_tools.join(", "));
                }
                println!();
                println!("System prompt:");
                println!("{}", spec.system_prompt);
            }
            None => {
                anyhow::bail!("persona '{}' not found. Run `agent007 persona list` to see available personas.", name);
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_output_covers_all_builtins() {
        let registry = PersonaRegistry::built_in();
        let personas = registry.list();
        assert_eq!(personas.len(), 15);
        let names: Vec<&str> = personas.iter().map(|p| p.name.as_str()).collect();
        for expected in &[
            "Researcher",
            "Architect",
            "Coder",
            "TestDesigner",
            "SecurityReviewer",
            "PerformanceEngineer",
            "DocumentationWriter",
            "DependencyManager",
            "DebugAgent",
            "CodeReviewer",
            "ExpertCoder",
            "UIUXDesigner",
            "DocsManager",
            "DevOpsEngineer",
            "DataEngineer",
        ] {
            assert!(
                names.contains(expected),
                "missing persona in list: {}",
                expected
            );
        }
    }

    #[test]
    fn show_known_persona_returns_spec() {
        let registry = PersonaRegistry::built_in();
        let spec = registry.get("Researcher");
        assert!(spec.is_some());
        let spec = spec.unwrap();
        assert!(!spec.system_prompt.is_empty());
    }

    #[test]
    fn show_unknown_persona_returns_none() {
        let registry = PersonaRegistry::built_in();
        let spec = registry.get("Ghost");
        assert!(spec.is_none());
    }
}
