use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;

use agent007_simulation::{SimulationPipeline, TemplateLoader};

use super::run::agent007_home;
use crate::config::Config;

#[derive(Parser, Debug)]
pub struct SimulateArgs {
    #[command(subcommand)]
    pub action: SimulateAction,
}

#[derive(Subcommand, Debug)]
pub enum SimulateAction {
    /// List all available simulation templates (built-in + user)
    List,
    /// Run a simulation template by name
    Run {
        /// Template name (e.g. wifi-rtt, wifi-roaming) or path to a .toml file
        template: String,
    },
    /// Validate a simulation template without running it
    Validate {
        /// Template name or path
        template: String,
    },
}

pub async fn execute(_config: Arc<Config>, args: SimulateArgs) -> Result<()> {
    let custom_dir = agent007_home().join("simulations").join("custom");
    let loader = TemplateLoader::with_custom_dir(custom_dir);

    match args.action {
        SimulateAction::List => {
            let names = loader.list();
            if names.is_empty() {
                println!("No simulation templates found.");
            } else {
                println!("{} template(s):", names.len());
                for name in &names {
                    println!("  {}", name);
                }
            }
        }

        SimulateAction::Run { template } => {
            let tmpl = loader.load(&template)?;
            println!("Running simulation: {}", tmpl.name);

            let memory_dir = agent007_home().join("memory");
            let memory = Arc::new(agent007_memory::store::MemoryStore::new(&memory_dir));

            let mock = Arc::new(agent007_models::MockProvider::new(
                "simulation output placeholder",
                "mock",
            )) as Arc<dyn agent007_models::ModelProvider>;

            let dispatcher = agent007_core::dispatcher::LocalDispatcher::new(16)
                as Arc<dyn agent007_core::dispatcher::Dispatcher>;

            let pipeline = SimulationPipeline {
                provider: mock,
                memory,
                dispatcher,
            };
            let report = pipeline.run(&tmpl).await?;

            println!("\n=== Simulation Report: {} ===", report.template_name);
            println!("Scenarios run : {}", report.scenarios_run);
            println!("Scenarios ok  : {}", report.scenarios_passed);
            println!("Scenarios fail: {}", report.scenarios_failed);
            if !report.failures.is_empty() {
                println!("\nFailures:");
                for f in &report.failures {
                    println!("  [{}] {}", f.scenario, f.reason);
                }
            }
        }

        SimulateAction::Validate { template } => {
            let tmpl = loader.load(&template)?;
            println!(
                "Template '{}' is valid ({} scenario(s)).",
                tmpl.name,
                tmpl.scenarios.len()
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_simulate_list() {
        let args = SimulateArgs::try_parse_from(["simulate", "list"]).unwrap();
        assert!(matches!(args.action, SimulateAction::List));
    }

    #[test]
    fn parse_simulate_run() {
        let args = SimulateArgs::try_parse_from(["simulate", "run", "wifi-rtt"]).unwrap();
        assert!(
            matches!(args.action, SimulateAction::Run { ref template } if template == "wifi-rtt")
        );
    }

    #[test]
    fn parse_simulate_validate() {
        let args = SimulateArgs::try_parse_from(["simulate", "validate", "wifi-roaming"]).unwrap();
        assert!(
            matches!(args.action, SimulateAction::Validate { ref template } if template == "wifi-roaming")
        );
    }
}
