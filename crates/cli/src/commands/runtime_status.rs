use std::sync::Arc;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use clap::{Args, ValueEnum};
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
    /// Filter sessions shown in the table. Counts still summarize the inspected window.
    #[arg(long, value_enum, default_value_t = StatusFilter::All)]
    pub state: StatusFilter,
    /// Refresh the compact table every N seconds until interrupted.
    #[arg(long, num_args = 0..=1, default_missing_value = "2")]
    pub watch: Option<u64>,
    /// Emit machine-readable JSON instead of the compact table.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum StatusFilter {
    All,
    Active,
    Blocked,
    Failed,
    Complete,
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
    let filter = args.state;

    if let Some(interval) = args.watch {
        if args.json {
            bail!("--watch cannot be combined with --json");
        }
        let interval = interval.clamp(1, 60);
        loop {
            print!("\x1B[2J\x1B[H");
            let snapshot = load_snapshot(limit, filter)?;
            print_snapshot(&snapshot, Some(interval));
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        }
    }

    let snapshot = load_snapshot(limit, filter)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        print_snapshot(&snapshot, None);
    }
    Ok(())
}

fn load_snapshot(limit: usize, filter: StatusFilter) -> Result<RuntimeStatusSnapshot> {
    let sessions_dir = status_sessions_dir();
    let mut snapshot = if sessions_dir.is_dir() {
        let store = RunStore::new(sessions_dir);
        build_snapshot(&store, limit)?
    } else {
        empty_snapshot()
    };
    apply_filter(&mut snapshot, filter);
    Ok(snapshot)
}

fn apply_filter(snapshot: &mut RuntimeStatusSnapshot, filter: StatusFilter) {
    if filter == StatusFilter::All {
        return;
    }
    snapshot.sessions.retain(|session| match filter {
        StatusFilter::All => true,
        StatusFilter::Active => matches!(
            session.lifecycle.as_str(),
            "running" | "ready" | "blocked" | "attention"
        ),
        StatusFilter::Blocked => session.lifecycle == "blocked",
        StatusFilter::Failed => session.lifecycle == "failed",
        StatusFilter::Complete => session.lifecycle == "complete",
    });
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

fn print_snapshot(snapshot: &RuntimeStatusSnapshot, watch_interval: Option<u64>) {
    println!(
        "agent007 runtime status · {}",
        snapshot.generated_at.format("%Y-%m-%d %H:%M:%SZ")
    );
    if let Some(interval) = watch_interval {
        println!("watching every {interval}s — press Ctrl-C to stop");
    }
    println!(
        "active={} running={} blocked={} failed={} complete={} total={} shown={}",
        snapshot.counts.active,
        snapshot.counts.running,
        snapshot.counts.blocked,
        snapshot.counts.failed,
        snapshot.counts.succeeded,
        snapshot.counts.total,
        snapshot.sessions.len()
    );
    println!();

    if snapshot.sessions.is_empty() {
        if snapshot.counts.total == 0 {
            println!("No recorded sessions yet.");
        } else {
            println!("No sessions match the current filter.");
        }
        return;
    }

    println!(
        "{:<9} {:<18} {:<18} {:<15} {:<14} {:<10} {}",
        "state", "session", "kind", "workflow", "runtime", "age", "hint"
    );
    println!("{}", "─".repeat(112));
    for session in &snapshot.sessions {
        let workflow = session
            .workflow
            .as_ref()
            .map(format_workflow)
            .unwrap_or_else(|| "—".to_string());
        println!(
            "{:<9} {:<18} {:<18} {:<15} {:<14} {:<10} {}",
            session.lifecycle,
            truncate(&session.id, 18),
            truncate(&session.kind, 18),
            truncate(&workflow, 15),
            truncate(&format_runtime(session), 14),
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

fn format_runtime(session: &RuntimeSessionRow) -> String {
    match session.provider.as_deref() {
        Some(provider) if !provider.is_empty() => format!("{}/{}", session.mode, provider),
        _ => session.mode.clone(),
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
    fn status_filter_keeps_requested_lifecycle_rows() {
        let mut snapshot = empty_snapshot();
        snapshot.sessions = vec![
            RuntimeSessionRow {
                id: "r1".to_string(),
                kind: "workflow".to_string(),
                task: "blocked".to_string(),
                status: "awaiting-approval".to_string(),
                lifecycle: "blocked".to_string(),
                mode: "hosted-mcp".to_string(),
                provider: None,
                age_seconds: 1,
                workflow: None,
                action_hint: "approval needed".to_string(),
                output_preview: None,
            },
            RuntimeSessionRow {
                id: "r2".to_string(),
                kind: "task".to_string(),
                task: "done".to_string(),
                status: "succeeded".to_string(),
                lifecycle: "complete".to_string(),
                mode: "standalone".to_string(),
                provider: Some("codex".to_string()),
                age_seconds: 2,
                workflow: None,
                action_hint: "review output".to_string(),
                output_preview: None,
            },
        ];

        apply_filter(&mut snapshot, StatusFilter::Blocked);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.sessions[0].id, "r1");
    }

    #[test]
    fn format_runtime_includes_provider_when_present() {
        let row = RuntimeSessionRow {
            id: "r1".to_string(),
            kind: "task".to_string(),
            task: "demo".to_string(),
            status: "running".to_string(),
            lifecycle: "running".to_string(),
            mode: "standalone".to_string(),
            provider: Some("codex".to_string()),
            age_seconds: 1,
            workflow: None,
            action_hint: "monitor".to_string(),
            output_preview: None,
        };
        assert_eq!(format_runtime(&row), "standalone/codex");
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
                state: StatusFilter::All,
                watch: None,
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
