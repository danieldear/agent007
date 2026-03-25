// crates/cli/src/commands/persona.rs
use std::sync::Arc;
use anyhow::Result;
use agent007_core::PersonaProvider;
use agent007_personas::PersonaRegistry;
use crate::PersonaAction;

/// Top-level dispatch for `agent007 persona <action>`.
pub async fn execute(_config: Arc<crate::config::Config>, action: PersonaAction) -> Result<()> {
    let personas_dir = crate::commands::run::agent007_home().join("personas");
    let registry = PersonaRegistry::load(&personas_dir).unwrap_or_else(|e| {
        tracing::warn!("failed to load persona overrides: {}", e);
        PersonaRegistry::built_in()
    });

    match action {
        PersonaAction::List => {
            let personas = registry.list();
            if personas.is_empty() {
                println!("No personas available.");
            } else {
                println!("{:<22} {:<10} {}", "NAME", "MODEL", "DESCRIPTION");
                println!("{}", "-".repeat(72));
                for p in personas {
                    println!("{:<22} {:<10} {}", p.name, p.preferred_model, p.description);
                }
            }
        }
        PersonaAction::Show { name } => {
            match registry.get(&name) {
                Some(spec) => {
                    println!("Name:            {}", spec.name);
                    println!("Model:           {}", spec.preferred_model);
                    println!("Description:     {}", spec.description);
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
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_output_covers_all_ten_builtins() {
        // We test the data layer directly — formatting is handled separately.
        let registry = PersonaRegistry::built_in();
        let personas = registry.list();
        assert_eq!(personas.len(), 10);
        let names: Vec<&str> = personas.iter().map(|p| p.name.as_str()).collect();
        for expected in &[
            "Researcher", "Architect", "Coder", "TestDesigner",
            "SecurityReviewer", "PerformanceEngineer", "DocumentationWriter",
            "DependencyManager", "DebugAgent", "CodeReviewer",
        ] {
            assert!(names.contains(expected), "missing persona in list: {}", expected);
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
