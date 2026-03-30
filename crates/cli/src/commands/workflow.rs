use anyhow::Result;
use clap::Subcommand;
use std::sync::Arc;
use super::run::{agent007_home, build_stack};
use crate::config::Config;

#[derive(Subcommand, Debug)]
pub enum WorkflowAction {
    /// Run a named workflow with an initial task
    Run {
        /// Workflow name (resolves under project-local or global `.agent007/workflows/<name>.toml`)
        name: String,
        /// Initial task input for {{task}} template variable
        #[arg(long)]
        task: String,
    },
    /// Resume a persisted workflow run from a prior session ID
    Resume {
        /// Session ID from `.agent007/sessions/<id>`
        #[arg(long)]
        session: String,
    },
    /// Record an approval decision for a pending workflow step
    Approve {
        /// Session ID from `.agent007/sessions/<id>`
        #[arg(long)]
        session: String,
        /// Step ID awaiting approval. Defaults to the current pending approval step.
        #[arg(long)]
        step: Option<String>,
        /// Decision: approve, deny, or edit
        #[arg(long)]
        decision: String,
        /// Edited content to use when decision=edit
        #[arg(long)]
        content: Option<String>,
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
                Ok(validated_dag) => {
                    println!("Workflow '{}' is valid.", name);
                    println!("Execution plan ({} batch(es)):", validated_dag.batches.len());
                    for (i, batch) in validated_dag.batches.iter().enumerate() {
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
            execute_workflow_run(
                config.clone(),
                def,
                task,
                "workflow-cli",
                None,
                Some(name),
            )
            .await?;
        }

        WorkflowAction::Resume { session } => {
            let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
            let request: agent007_workflows::WorkflowRunRequest = store
                .read_json_artifact(&session, "workflow-request.json")?;
            let workflow_ref = store
                .read_json_artifact_optional::<agent007_workflows::WorkflowSourceRef>(
                    &session,
                    "workflow-source.json",
                )?
                .map(|source| source.workflow_ref)
                .unwrap_or_else(|| request.workflow.clone());
            let state: agent007_workflows::WorkflowRunState = store
                .read_json_artifact(&session, "workflow-state.json")?;
            let def = loader.load_named(&workflow_ref)?;
            println!(
                "Resuming workflow '{}' from session {} ({}/{} steps complete)",
                request.workflow,
                session,
                state.steps_completed,
                state.steps_total,
            );
            execute_workflow_run(
                config.clone(),
                def,
                request.task,
                "workflow-resume-cli",
                Some(state),
                Some(workflow_ref),
            )
            .await?;
        }

        WorkflowAction::Approve {
            session,
            step,
            decision,
            content,
        } => {
            let store = agent007_core::RunStore::new(agent007_home().join("sessions"));
            let mut state: agent007_workflows::WorkflowRunState = store
                .read_json_artifact(&session, "workflow-state.json")?;
            let step_id = step
                .or_else(|| state.pending_approval.as_ref().map(|pending| pending.step_id.clone()))
                .ok_or_else(|| anyhow::anyhow!("no pending approval found in session {}", session))?;
            let decision = parse_approval_decision(&decision, content)?;
            state.record_approval_decision(&step_id, decision);
            store.write_json_artifact(&session, "workflow-state.json", &state)?;
            println!(
                "Recorded approval decision for step '{}' in session {}. Run `agent007 workflow resume --session {}` to continue.",
                step_id, session, session,
            );
        }
    }

    Ok(())
}

pub async fn execute_workflow_run(
    config: Arc<Config>,
    def: agent007_workflows::WorkflowDef,
    task: String,
    run_kind: &str,
    resume_state: Option<agent007_workflows::WorkflowRunState>,
    workflow_ref: Option<String>,
) -> Result<()> {
    let stack = build_stack(&config).await?;
    let run = stack
        .run_store
        .create_run(run_kind, &format!("{}: {}", def.name, task), "standalone", None)?;
    if let Some(workflow_ref) = workflow_ref {
        stack.run_store.write_json_artifact(
            &run.id,
            "workflow-source.json",
            &agent007_workflows::WorkflowSourceRef { workflow_ref },
        )?;
    }
    let runner = match resume_state {
        Some(state) => stack
            .workflow_runner
            .resume_from(stack.run_store.clone(), run.id.clone(), state),
        None => stack
            .workflow_runner
            .for_run(stack.run_store.clone(), run.id.clone()),
    };

    match runner.run(&def, &task).await {
        Ok(result) => {
            let _ = stack.run_store.finish_run(
                &run.id,
                true,
                format!("workflow '{}' completed", def.name),
            );
            print_workflow_result(&def.name, &result);
            println!("Session: {}", run.id);
            Ok(())
        }
        Err(error) => {
            match &error {
                agent007_workflows::WorkflowError::ApprovalRequired { id } => {
                    let _ = stack.run_store.finish_run_with_status(
                        &run.id,
                        agent007_core::run_store::RunStatus::AwaitingApproval,
                        format!("approval required for step '{}'", id),
                    );
                    eprintln!(
                        "Workflow '{}' is waiting for approval on step '{}'. Session: {}",
                        def.name, id, run.id,
                    );
                }
                _ => {
                    let _ = stack.run_store.finish_run(&run.id, false, error.to_string());
                    eprintln!("Workflow '{}' failed: {}", def.name, error);
                }
            }
            std::process::exit(1);
        }
    }
}

fn parse_approval_decision(
    decision: &str,
    content: Option<String>,
) -> Result<agent007_workflows::approval::ApprovalDecision> {
    use agent007_workflows::approval::{ApprovalDecision, ApprovalDecisionKind};

    let kind = match decision.trim().to_lowercase().as_str() {
        "approve" | "approved" | "yes" | "y" => ApprovalDecisionKind::Approve,
        "deny" | "denied" | "no" | "n" => ApprovalDecisionKind::Deny,
        "edit" | "edited" => ApprovalDecisionKind::Edit,
        other => {
            anyhow::bail!("unknown approval decision '{}'; use approve, deny, or edit", other);
        }
    };

    if kind == ApprovalDecisionKind::Edit && content.as_deref().unwrap_or("").trim().is_empty() {
        anyhow::bail!("--content is required when decision=edit");
    }

    Ok(ApprovalDecision { decision: kind, content })
}

fn print_workflow_result(name: &str, result: &agent007_workflows::WorkflowResult) {
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
