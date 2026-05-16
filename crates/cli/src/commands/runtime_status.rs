use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Args;
use serde::Serialize;

use crate::config::Config;
use agent007_core::paths::agent007_write_home;
use agent007_core::{RunMetadata, RunStatus, RunStore};
use agent007_workflows::{WorkflowRunState, WorkflowStepStatus};

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Number of recent sessions to inspect.
    #[arg(long, short = 'n', default_value_t = 12)]
    pub limit: usize,
    /// Emit machine-readable JSON instead of the compact table.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeStatusSnapshot {
    generated_at: DateTime<Utc>,
    counts: RuntimeStatusCounts,
    sessions: Vec<RuntimeSessionRow>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct RuntimeStatusCounts {
    total: usize,
    active: usize,
    running: usize,
    blocked: usize,
    failed: usize,
    succeeded: usize,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeSessionRow {
    id: String,
    kind: String,
    task: String,
    status: String,
    lifecycle: String,
    mode: String,
    provider: Option<String>,
    age_seconds: i64,
    workflow: Option<WorkflowRow>,
    action_hint: String,
    output_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowRow {
    workflow: String,
    status: String,
    completed_steps: usize,
    total_steps: usize,
    running_steps: Vec<String>,
    pending_approval_step: Option<String>,
    last_error: Option<String>,
}

pub async fn execute(_config: Arc<Config>, args: StatusArgs) -> Result<()> {
    let limit = args.limit.clamp(1, 100);
    let store = RunStore::new(agent007_write_home().join("sessions"));
    let snapshot = build_snapshot(&store, limit)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        print_snapshot(&snapshot);
    }
    Ok(())
}

fn build_snapshot(store: &RunStore, limit: usize) -> Result<RuntimeStatusSnapshot> {
    let runs = store.list_runs(limit)?;
    let mut counts = RuntimeStatusCounts {
        total: runs.len(),
        ..RuntimeStatusCounts::default()
    };
    let sessions = runs
        .into_iter()
        .map(|run| {
            let workflow = store
                .read_json_artifact_optional::<WorkflowRunState>(&run.id, "workflow-state.json")
                .ok()
                .flatten()
                .map(workflow_row);
            match run.status {
                RunStatus::Running => {
                    counts.active += 1;
                    counts.running += 1;
                }
                RunStatus::AwaitingApproval => {
                    counts.active += 1;
                    counts.blocked += 1;
                }
                RunStatus::Failed => counts.failed += 1,
                RunStatus::Succeeded => counts.succeeded += 1,
            }
            session_row(run, workflow)
        })
        .collect::<Vec<_>>();

    Ok(RuntimeStatusSnapshot {
        generated_at: Utc::now(),
        counts,
        sessions,
    })
}

fn workflow_row(state: WorkflowRunState) -> WorkflowRow {
    let running_steps = state
        .steps
        .iter()
        .filter(|step| matches!(step.status, WorkflowStepStatus::Running))
        .map(|step| step.id.clone())
        .collect::<Vec<_>>();
    WorkflowRow {
        workflow: state.workflow,
        status: serde_json::to_value(state.status)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "running".to_string()),
        completed_steps: state.steps_completed,
        total_steps: state.steps_total,
        running_steps,
        pending_approval_step: state.pending_approval.map(|approval| approval.step_id),
        last_error: state.last_error,
    }
}

fn session_row(run: RunMetadata, workflow: Option<WorkflowRow>) -> RuntimeSessionRow {
    let age_seconds = (Utc::now() - run.started_at).num_seconds().max(0);
    let lifecycle = lifecycle(&run.status, workflow.as_ref()).to_string();
    let action_hint = action_hint(&run.status, workflow.as_ref()).to_string();
    RuntimeSessionRow {
        id: run.id,
        kind: run.kind,
        task: run.task,
        status: run_status_label(&run.status).to_string(),
        lifecycle,
        mode: run.mode,
        provider: run.provider,
        age_seconds,
        workflow,
        action_hint,
        output_preview: run.output_preview,
    }
}

fn lifecycle(status: &RunStatus, workflow: Option<&WorkflowRow>) -> &'static str {
    match status {
        RunStatus::Running => {
            if workflow.and_then(|w| w.last_error.as_ref()).is_some() {
                "attention"
            } else if workflow
                .map(|w| !w.running_steps.is_empty())
                .unwrap_or(false)
            {
                "running"
            } else {
                "running"
            }
        }
        RunStatus::AwaitingApproval => "blocked",
        RunStatus::Succeeded => "complete",
        RunStatus::Failed => "failed",
    }
}

fn action_hint(status: &RunStatus, workflow: Option<&WorkflowRow>) -> &'static str {
    if let Some(workflow) = workflow {
        if workflow.pending_approval_step.is_some() || matches!(status, RunStatus::AwaitingApproval)
        {
            return "approve or resume workflow";
        }
        if workflow.last_error.is_some() || matches!(status, RunStatus::Failed) {
            return "inspect workflow error";
        }
        if !workflow.running_steps.is_empty() {
            return "monitor running steps";
        }
    }
    match status {
        RunStatus::Running => "monitor run",
        RunStatus::AwaitingApproval => "approval needed",
        RunStatus::Succeeded => "review output",
        RunStatus::Failed => "inspect failure",
    }
}

fn run_status_label(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::AwaitingApproval => "awaiting-approval",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}

fn print_snapshot(snapshot: &RuntimeStatusSnapshot) {
    println!("agent007 runtime status");
    println!(
        "active={} running={} blocked={} failed={} total={}",
        snapshot.counts.active,
        snapshot.counts.running,
        snapshot.counts.blocked,
        snapshot.counts.failed,
        snapshot.counts.total
    );
    println!();

    if snapshot.sessions.is_empty() {
        println!("No recorded sessions yet.");
        return;
    }

    println!(
        "{:<9} {:<18} {:<18} {:<15} {:<10} {}",
        "state", "session", "kind", "workflow", "age", "hint"
    );
    println!("{}", "─".repeat(96));
    for session in &snapshot.sessions {
        let workflow = session
            .workflow
            .as_ref()
            .map(format_workflow)
            .unwrap_or_else(|| "—".to_string());
        println!(
            "{:<9} {:<18} {:<18} {:<15} {:<10} {}",
            session.lifecycle,
            truncate(&session.id, 18),
            truncate(&session.kind, 18),
            truncate(&workflow, 15),
            format_age(session.age_seconds),
            session.action_hint
        );
        println!("  task: {}", truncate(&session.task.replace('\n', " "), 88));
        if let Some(error) = session
            .workflow
            .as_ref()
            .and_then(|workflow| workflow.last_error.as_ref())
        {
            println!("  error: {}", truncate(error, 88));
        } else if let Some(preview) = &session.output_preview {
            println!("  output: {}", truncate(&preview.replace('\n', " "), 87));
        }
    }
}

fn format_workflow(workflow: &WorkflowRow) -> String {
    let mut value = format!(
        "{} {}/{}",
        workflow.workflow, workflow.completed_steps, workflow.total_steps
    );
    if let Some(step) = &workflow.pending_approval_step {
        value.push_str(&format!(" gate:{step}"));
    } else if !workflow.running_steps.is_empty() {
        value.push_str(&format!(" run:{}", workflow.running_steps.join(",")));
    }
    value
}

fn format_age(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h{}m", seconds / 3_600, (seconds % 3_600) / 60)
    } else {
        format!("{}d{}h", seconds / 86_400, (seconds % 86_400) / 3_600)
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_age_uses_compact_units() {
        assert_eq!(format_age(12), "12s");
        assert_eq!(format_age(600), "10m");
        assert_eq!(format_age(3_900), "1h5m");
        assert_eq!(format_age(90_000), "1d1h");
    }

    #[test]
    fn truncate_preserves_short_values_and_ellipsizes_long_values() {
        assert_eq!(truncate("hello", 8), "hello");
        assert_eq!(truncate("hello world", 8), "hello w…");
    }
}
