use anyhow::Result;
use clap::Subcommand;
use std::sync::Arc;
use crate::config::Config;
use crate::commands::run::{agent007_home, build_stack};

#[derive(Subcommand, Debug)]
pub enum WorkflowAction {
    /// Run a named workflow with an initial task
    Run {
        /// Workflow name (resolves ~/.agent007/workflows/<name>.toml)
        name: String,
        /// Initial task input for {{task}} template variable
        #[arg(long)]
        task: String,
    },
    /// List all available workflows
    List,
    /// Validate a workflow DAG without running it
    Validate {
        /// Workflow name
        name: String,
    },
    /// Show a workflow's steps and dependencies
    Show {
        /// Workflow name
        name: String,
    },
}

pub async fn execute(config: Arc<Config>, action: WorkflowAction) -> Result<()> {
    let workflows_dir = agent007_home().join("workflows");
    let loader = agent007_workflows::WorkflowLoader::new(workflows_dir.clone());

    match action {
        WorkflowAction::List => {
            let names = loader.list_names()?;
            if names.is_empty() {
                println!("No workflows found in {}", workflows_dir.display());
                println!("Add TOML files to {} to create workflows.", workflows_dir.display());
            } else {
                println!("Available workflows (in {}):", workflows_dir.display());
                for name in &names {
                    let desc = loader.load_named(name)
                        .ok()
                        .and_then(|d| d.description)
                        .unwrap_or_default();
                    if desc.is_empty() {
                        println!("  {}", name);
                    } else {
                        println!("  {} — {}", name, desc);
                    }
                }
            }
        }

        WorkflowAction::Validate { name } => {
            let def = loader.load_named(&name)?;
            let stack = build_stack(&config).await?;
            let runner = stack.workflow_runner.clone();
            match runner.validate(&def) {
                Ok(batches) => {
                    println!("Workflow '{}' is valid.", name);
                    println!("Execution plan ({} batch(es)):", batches.len());
                    for (i, batch) in batches.iter().enumerate() {
                        println!("  Batch {}: [{}]", i + 1, batch.join(", "));
                    }
                }
                Err(e) => {
                    eprintln!("Workflow '{}' is invalid: {}", name, e);
                    std::process::exit(1);
                }
            }
        }

        WorkflowAction::Show { name } => {
            let def = loader.load_named(&name)?;
            println!("Workflow: {}", def.name);
            if let Some(desc) = &def.description {
                println!("Description: {}", desc);
            }
            println!("\nSteps:");
            for step in &def.steps {
                println!("  [{}] agent={}", step.id, step.agent);
                if let Some(m) = &step.model {
                    println!("       model={}", m);
                }
                if let Some(inputs) = &step.inputs {
                    println!("       inputs=[{}]", inputs.join(", "));
                }
                if let Some(deps) = &step.depends_on {
                    println!("       depends_on=[{}]", deps.join(", "));
                }
                if let Some(out) = &step.output {
                    println!("       output={}", out);
                }
                if step.requires_approval == Some(true) {
                    println!("       requires_approval=true");
                }
            }
            if let Some(budget) = &def.budget {
                println!("\nBudget:");
                if let Some(t) = budget.max_tokens_per_session {
                    println!("  max_tokens_per_session={}", t);
                }
                if let Some(u) = budget.max_usd_per_task {
                    println!("  max_usd_per_task=${:.2}", u);
                }
                if let Some(pct) = budget.alert_at_percent {
                    println!("  alert_at_percent={}%", pct);
                }
                if let Some(mode) = &budget.on_exceed {
                    println!("  on_exceed={}", mode);
                }
            }
        }

        WorkflowAction::Run { name, task } => {
            let def = loader.load_named(&name)?;
            println!("Running workflow '{}' with task: {}", name, task);

            let stack = build_stack(&config).await?;
            let runner = stack.workflow_runner.clone();

            match runner.run(&def, &task).await {
                Ok(result) => {
                    println!(
                        "\nWorkflow '{}' completed: {}/{} steps",
                        name, result.steps_completed, result.steps_total
                    );
                    println!(
                        "Budget used: {} tokens, ${:.6}",
                        result.budget_used.tokens, result.budget_used.estimated_usd
                    );
                    if !result.outputs.is_empty() {
                        println!("\nOutputs:");
                        for (key, value) in &result.outputs {
                            let preview = if value.len() > 200 {
                                format!("{}... ({} chars)", &value[..200], value.len())
                            } else {
                                value.clone()
                            };
                            println!("  [{}]:\n{}\n", key, preview);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Workflow '{}' failed: {}", name, e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
