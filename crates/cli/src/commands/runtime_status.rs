use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Args;
use serde::Serialize;

use crate::config::Config;
use agent007_core::paths::{agent007_global_home, agent007_project_home};
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
    pending_steps: usize,
    pending_approval_step: Option<String>,
    last_error: Option<String>,
}

pub async fn execute(_config: Arc<Config>, args: StatusArgs) -> Result<()> {
    let limit = args.limit.clamp(1, 100);
    let sessions_dir = status_sessions_dir();
    let snapshot = if sessions_dir.is_dir() {
        let store = RunStore::new(sessions_dir);
        build_snapshot(&store, limit)?
    } else {
        empty_snapshot()
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        print_snapshot(&snapshot);
    }
    Ok(())
}

fn status_sessions_dir() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        return std::path::PathBuf::from(home).join("sessions");
    }
    if let Some(project_home) = agent007_project_home() {
        return project_home.join("sessions");
    }
    agent007_global_home().join("sessions")
}

fn empty_snapshot() -> RuntimeStatusSnapshot {
    RuntimeStatusSnapshot {
        generated_at: Utc::now(),
        counts: RuntimeStatusCounts::default(),
        sessions: Vec::new(),
    }
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
    let pending_steps = state
        .steps
        .iter()
        .filter(|step| matches!(step.status, WorkflowStepStatus::Pending))
        .count();
    WorkflowRow {
        workflow: state.workflow,
        status: serde_json::to_value(state.status)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "running".to_string()),
        completed_steps: state.steps_completed,
        total_steps: state.steps_total,
        running_steps,
        pending_steps,
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
            } else if workflow
                .map(|w| w.pending_steps > 0 && w.completed_steps < w.total_steps)
                .unwrap_or(false)
            {
                "ready"
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
        if workflow.pending_steps > 0 && workflow.completed_steps < workflow.total_steps {
            return "submit ready steps";
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
        println!("  task: {}", truncate(&compact_line(&session.task), 88));
        if let Some(error) = session
            .workflow
            .as_ref()
            .and_then(|workflow| workflow.last_error.as_ref())
        {
            println!("  error: {}", truncate(&compact_line(error), 88));
        } else if let Some(preview) = &session.output_preview {
            println!("  output: {}", truncate(&compact_line(preview), 87));
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
    } else if workflow.pending_steps > 0 && workflow.completed_steps < workflow.total_steps {
        value.push_str(&format!(" ready:{}", workflow.pending_steps));
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

fn compact_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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

    #[test]
    fn compact_line_normalizes_embedded_whitespace() {
        assert_eq!(
            compact_line("first line\nsecond\tline"),
            "first line second line"
        );
    }

    #[test]
    fn build_snapshot_counts_ready_workflow_sessions() {
        use agent007_workflows::types::{StepDef, StepType, WorkflowDef};

        let tmp = tempfile::tempdir().unwrap();
        let store = RunStore::new(tmp.path().join("sessions"));
        let run = store
            .create_run(
                "workflow",
                "ship compact status",
                "hosted-mcp",
                Some("gpt-test"),
            )
            .unwrap();
        let workflow = WorkflowDef {
            name: "ship".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "plan".to_string(),
                    agent: "Planner".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: None,
                    prompt: Some("plan".to_string()),
                    skill: None,
                    output: Some("plan".to_string()),
                    requires_approval: None,
                    approval_prompt: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                    workflow: None,
                    extract: None,
                    cache: false,
                },
                StepDef {
                    id: "implement".to_string(),
                    agent: "Coder".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: Some(vec!["plan".to_string()]),
                    prompt: Some("implement".to_string()),
                    skill: None,
                    output: Some("code".to_string()),
                    requires_approval: None,
                    approval_prompt: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                    workflow: None,
                    extract: None,
                    cache: false,
                },
            ],
            budget: None,
            reliability: None,
            eval_gate: None,
        };
        let mut state = WorkflowRunState::new(&workflow, "ship compact status");
        state.mark_step_completed(&workflow.steps[0], "done");
        store
            .write_json_artifact(&run.id, "workflow-state.json", &state)
            .unwrap();

        let snapshot = build_snapshot(&store, 10).unwrap();
        assert_eq!(snapshot.counts.total, 1);
        assert_eq!(snapshot.counts.active, 1);
        assert_eq!(snapshot.counts.running, 1);
        assert_eq!(snapshot.sessions.len(), 1);
        let session = &snapshot.sessions[0];
        assert_eq!(session.lifecycle, "ready");
        assert_eq!(session.action_hint, "submit ready steps");
        let workflow = session.workflow.as_ref().unwrap();
        assert_eq!(workflow.completed_steps, 1);
        assert_eq!(workflow.total_steps, 2);
        assert_eq!(workflow.pending_steps, 1);
    }

    #[tokio::test]
    async fn status_execute_does_not_create_project_home_when_no_sessions_exist() {
        let _guard = crate::test_support::env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"empty\"\n",
        )
        .unwrap();
        std::env::remove_var("AGENT007_HOME");
        std::env::set_current_dir(tmp.path()).unwrap();

        execute(
            Arc::new(Config::default()),
            StatusArgs {
                limit: 3,
                json: true,
            },
        )
        .await
        .unwrap();

        assert!(
            !tmp.path().join(".agent007").exists(),
            "read-only status must not create a project .agent007 directory"
        );
        std::env::set_current_dir(original_dir).unwrap();
    }
}
