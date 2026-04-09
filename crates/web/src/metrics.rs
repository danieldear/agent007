use std::path::Path;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use serde::Serialize;
use chrono::{DateTime, Utc};

use agent007_core::{
    events::AgentEvent,
    run_store::{RunStatus, RunStore, RunTokenSummary},
};
use agent007_learning::LearningEvent;

pub type MetricsState = Arc<Mutex<DashboardMetrics>>;

#[derive(Debug, Clone, Serialize)]
pub struct DashboardMetrics {
    pub active_agents: u32,
    pub running_tasks: u32,
    pub completed_tasks: u32,
    pub failed_tasks: u32,

    pub total_tokens: u64,
    pub estimated_usd: f64,
    pub session_requests: u32,

    pub avg_reward: f64,
    pub feedback_count: u32,
    pub prompt_improvements: u32,
    reward_sum: f64,

    pub skills_count: u32,
    pub workflows_count: u32,
    pub personas_count: u32,
    pub memory_keys: u32,

    pub started_at: DateTime<Utc>,
    pub local_execution_available: bool,
    pub runtime_mode: String,
    pub model_provider: String,

    pub recent_tasks: VecDeque<TaskLogEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskLogEntry {
    pub id: String,
    pub task: String,
    pub status: String,
    pub agent: String,
    pub model: String,
    pub tokens: u64,
    pub started_at: String,
    pub finished_at: Option<String>,
}

const MAX_RECENT_TASKS: usize = 50;
const TOKEN_PRICE_PER_TOKEN_USD: f64 = 0.000_002;

impl DashboardMetrics {
    pub fn new() -> Self {
        let local_execution_available = std::env::var("ANTHROPIC_API_KEY")
            .map(|k| !k.is_empty())
            .unwrap_or(false)
            || std::env::var("OPENAI_API_KEY")
                .map(|k| !k.is_empty())
                .unwrap_or(false);

        Self {
            active_agents: 0,
            running_tasks: 0,
            completed_tasks: 0,
            failed_tasks: 0,
            total_tokens: 0,
            estimated_usd: 0.0,
            session_requests: 0,
            avg_reward: 0.0,
            feedback_count: 0,
            prompt_improvements: 0,
            reward_sum: 0.0,
            skills_count: 0,
            workflows_count: 0,
            personas_count: 0,
            memory_keys: 0,
            started_at: Utc::now(),
            local_execution_available,
            runtime_mode: if std::env::var("AGENT007_DRY_RUN").is_ok() {
                "dry-run".to_string()
            } else if std::env::var("OPENAI_API_KEY").map(|k| !k.is_empty()).unwrap_or(false)
                || std::env::var("ANTHROPIC_API_KEY").map(|k| !k.is_empty()).unwrap_or(false)
            {
                "standalone".to_string()
            } else {
                "hosted-mcp".to_string()
            },
            model_provider: if std::env::var("OPENAI_API_KEY").map(|k| !k.is_empty()).unwrap_or(false) {
                std::env::var("OPENAI_MODEL").ok()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "codex".to_string())
            } else if std::env::var("ANTHROPIC_API_KEY").map(|k| !k.is_empty()).unwrap_or(false) {
                std::env::var("ANTHROPIC_MODEL").ok()
                    .or_else(|| std::env::var("CLAUDE_MODEL").ok())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "claude".to_string())
            } else {
                std::env::var("AGENT007_HOST_MODEL").ok()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "hosted-mcp".to_string())
            },
            recent_tasks: VecDeque::new(),
        }
    }

    pub fn with_runtime(
        local_execution_available: bool,
        runtime_mode: impl Into<String>,
        model_provider: impl Into<String>,
    ) -> Self {
        let mut metrics = Self::new();
        metrics.local_execution_available = local_execution_available;
        metrics.runtime_mode = runtime_mode.into();
        metrics.model_provider = model_provider.into();
        metrics
    }

    pub fn process_agent_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::TaskAssigned { agent_id, task } => {
                self.running_tasks += 1;
                self.active_agents = self.active_agents.max(1);
                let entry = TaskLogEntry {
                    id: format!("{}", agent_id),
                    task: task.description.chars().take(120).collect(),
                    status: "running".to_string(),
                    agent: format!("{}", agent_id),
                    model: self.model_provider.clone(),
                    tokens: 0,
                    started_at: Utc::now().format("%H:%M:%S").to_string(),
                    finished_at: None,
                };
                self.recent_tasks.push_back(entry);
                if self.recent_tasks.len() > MAX_RECENT_TASKS {
                    self.recent_tasks.pop_front();
                }
            }
            AgentEvent::TaskCompleted { agent_id, .. } => {
                self.running_tasks = self.running_tasks.saturating_sub(1);
                self.completed_tasks += 1;
                let aid = format!("{}", agent_id);
                if let Some(entry) = self.recent_tasks.iter_mut().rev().find(|e| e.id == aid && e.status == "running") {
                    entry.status = "completed".to_string();
                    entry.finished_at = Some(Utc::now().format("%H:%M:%S").to_string());
                }
            }
            AgentEvent::ModelRequest { token_estimate, provider, .. } => {
                self.total_tokens += *token_estimate as u64;
                self.estimated_usd = self.total_tokens as f64 * TOKEN_PRICE_PER_TOKEN_USD;
                self.session_requests += 1;
                // Credit tokens to the most recent running task and update its model.
                if let Some(entry) = self.recent_tasks.iter_mut().rev().find(|e| e.status == "running") {
                    entry.tokens += *token_estimate as u64;
                    entry.model = provider.clone();
                }
            }
            AgentEvent::ToolCall { .. } => {}
            AgentEvent::ToolCallResult { .. } => {}
            AgentEvent::MemoryWrite { .. } => {}
            AgentEvent::HookFired { .. } => {}
        }
    }

    pub fn process_learning_event(&mut self, event: &LearningEvent) {
        match event {
            LearningEvent::FeedbackRecorded { reward, .. } => {
                self.reward_sum += *reward as f64;
                self.feedback_count += 1;
                self.avg_reward = self.reward_sum / self.feedback_count as f64;
            }
            LearningEvent::PromptImproved { .. } => {
                self.prompt_improvements += 1;
            }
            LearningEvent::OptimizerTriggered { .. } => {}
        }
    }

    pub fn update_inventory(&mut self, skills: u32, workflows: u32, personas: u32, memory_keys: u32) {
        self.skills_count = skills;
        self.workflows_count = workflows;
        self.personas_count = personas;
        self.memory_keys = memory_keys;
    }
}

pub fn snapshot_with_shared_state(
    base: DashboardMetrics,
    home: impl AsRef<Path>,
) -> DashboardMetrics {
    let mut snapshot = base;
    hydrate_from_run_store(&mut snapshot, &RunStore::new(home.as_ref().join("sessions")));
    snapshot
}

fn hydrate_from_run_store(metrics: &mut DashboardMetrics, store: &RunStore) {
    let runs = match store.list_runs(200) {
        Ok(runs) => runs,
        Err(_) => return,
    };

    metrics.running_tasks = 0;
    metrics.completed_tasks = 0;
    metrics.failed_tasks = 0;
    metrics.total_tokens = 0;
    metrics.estimated_usd = 0.0;
    metrics.session_requests = 0;
    metrics.recent_tasks.clear();

    for run in runs.iter().rev() {
        match run.status {
            RunStatus::Running | RunStatus::AwaitingApproval => {
                metrics.running_tasks += 1;
            }
            RunStatus::Succeeded => {
                metrics.completed_tasks += 1;
            }
            RunStatus::Failed => {
                metrics.failed_tasks += 1;
            }
        }

        let (tokens, requests) = load_run_token_totals(store, &run.id);
        metrics.total_tokens += tokens;
        metrics.session_requests += requests;

        let provider_label = run.provider.clone().unwrap_or_else(|| run.mode.clone());
        metrics.recent_tasks.push_back(TaskLogEntry {
            id: run.id.clone(),
            task: run.task.chars().take(120).collect(),
            status: run_status_label(&run.status).to_string(),
            agent: run.mode.clone(),
            model: provider_label,
            tokens,
            started_at: run.started_at.format("%H:%M:%S").to_string(),
            finished_at: run
                .finished_at
                .map(|value| value.format("%H:%M:%S").to_string()),
        });
        if metrics.recent_tasks.len() > MAX_RECENT_TASKS {
            metrics.recent_tasks.pop_front();
        }
    }

    metrics.active_agents = metrics.running_tasks.max(metrics.active_agents);
    metrics.estimated_usd = metrics.total_tokens as f64 * TOKEN_PRICE_PER_TOKEN_USD;
}

fn load_run_token_totals(store: &RunStore, run_id: &str) -> (u64, u32) {
    if let Ok(Some(summary)) =
        store.read_json_artifact_optional::<RunTokenSummary>(run_id, "token-summary.json")
    {
        return (summary.tokens, summary.requests);
    }

    let detail = match store.load_run(run_id) {
        Ok(detail) => detail,
        Err(_) => return (0, 0),
    };

    let mut tokens = 0u64;
    let mut requests = 0u32;
    for entry in detail.entries {
        if entry.kind != "agent-event" {
            continue;
        }
        if let Ok(AgentEvent::ModelRequest { token_estimate, .. }) =
            serde_json::from_value::<AgentEvent>(entry.payload)
        {
            tokens += token_estimate as u64;
            requests += 1;
        }
    }

    (tokens, requests)
}

fn run_status_label(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::AwaitingApproval => "awaiting-approval",
        RunStatus::Succeeded => "completed",
        RunStatus::Failed => "failed",
    }
}

pub fn new_metrics_state() -> MetricsState {
    Arc::new(Mutex::new(DashboardMetrics::new()))
}

pub fn new_metrics_state_with_runtime(
    local_execution_available: bool,
    runtime_mode: impl Into<String>,
    model_provider: impl Into<String>,
) -> MetricsState {
    Arc::new(Mutex::new(DashboardMetrics::with_runtime(
        local_execution_available,
        runtime_mode,
        model_provider,
    )))
}

/// Spawn a background task that subscribes to both dispatchers and updates metrics.
pub fn spawn_metrics_collector(
    metrics: MetricsState,
    dispatcher: Arc<agent007_core::dispatcher::LocalDispatcher>,
    learning_dispatcher: Arc<agent007_learning::LearningDispatcher>,
) {
    tokio::spawn(async move {
        use agent007_core::dispatcher::Dispatcher;
        use futures::StreamExt;

        let mut agent_stream = match dispatcher.subscribe().await {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut learning_stream = learning_dispatcher.subscribe();

        loop {
            tokio::select! {
                Some(event) = agent_stream.next() => {
                    metrics.lock().await.process_agent_event(&event);
                }
                Some(event) = learning_stream.next() => {
                    metrics.lock().await.process_learning_event(&event);
                }
                else => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::types::{AgentId, PromptRef};
    use agent007_core::{Task, run_store::RunStore};

    #[test]
    fn new_metrics_has_zero_counters() {
        let m = DashboardMetrics::new();
        assert_eq!(m.active_agents, 0);
        assert_eq!(m.running_tasks, 0);
        assert_eq!(m.completed_tasks, 0);
        assert_eq!(m.total_tokens, 0);
    }

    #[test]
    fn process_model_request_updates_tokens() {
        let mut m = DashboardMetrics::new();
        m.process_agent_event(&AgentEvent::ModelRequest {
            provider: "claude".to_string(),
            prompt_ref: PromptRef::new(),
            token_estimate: 500,
        });
        assert_eq!(m.total_tokens, 500);
        assert_eq!(m.session_requests, 1);
        assert!(m.estimated_usd > 0.0);
    }

    #[test]
    fn process_task_lifecycle() {
        let mut m = DashboardMetrics::new();
        let aid = AgentId::new();
        let task = Task::new("test task");

        m.process_agent_event(&AgentEvent::TaskAssigned {
            agent_id: aid.clone(),
            task: task.clone(),
        });
        assert_eq!(m.running_tasks, 1);
        assert_eq!(m.recent_tasks.len(), 1);

        m.process_agent_event(&AgentEvent::TaskCompleted {
            agent_id: aid,
            result: agent007_core::task::TaskResult::success(task.id, "done".to_string()),
            skill_name: None,
            model: None,
        });
        assert_eq!(m.running_tasks, 0);
        assert_eq!(m.completed_tasks, 1);
    }

    #[test]
    fn process_feedback_updates_avg_reward() {
        let mut m = DashboardMetrics::new();
        m.process_learning_event(&LearningEvent::FeedbackRecorded {
            agent_id: AgentId::new(),
            reward: 0.8,
        });
        m.process_learning_event(&LearningEvent::FeedbackRecorded {
            agent_id: AgentId::new(),
            reward: 0.6,
        });
        assert_eq!(m.feedback_count, 2);
        assert!((m.avg_reward - 0.7).abs() < 0.001);
    }

    #[test]
    fn recent_tasks_capped_at_max() {
        let mut m = DashboardMetrics::new();
        for i in 0..60 {
            m.process_agent_event(&AgentEvent::TaskAssigned {
                agent_id: AgentId::new(),
                task: Task::new(&format!("task {i}")),
            });
        }
        assert_eq!(m.recent_tasks.len(), MAX_RECENT_TASKS);
    }

    #[test]
    fn shared_snapshot_hydrates_recent_runs_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path().join("sessions"));

        let run = store
            .create_run("task", "ship auth", "hosted-mcp", Some("codex"))
            .unwrap();
        store
            .append_note(
                &run.id,
                "agent-event",
                serde_json::to_value(AgentEvent::ModelRequest {
                    provider: "codex".to_string(),
                    prompt_ref: PromptRef::new(),
                    token_estimate: 321,
                })
                .unwrap(),
            )
            .unwrap();
        store
            .finish_run_with_status(&run.id, RunStatus::Succeeded, "done")
            .unwrap();

        let pending = store
            .create_run("workflow", "approve deploy", "hosted-mcp", None)
            .unwrap();
        store
            .update_run_status(&pending.id, RunStatus::AwaitingApproval, Some("waiting".to_string()))
            .unwrap();

        let snapshot = snapshot_with_shared_state(
            DashboardMetrics::with_runtime(false, "hosted-mcp", "hosted-mcp"),
            dir.path(),
        );

        assert_eq!(snapshot.completed_tasks, 1);
        assert_eq!(snapshot.running_tasks, 1);
        assert_eq!(snapshot.session_requests, 1);
        assert_eq!(snapshot.total_tokens, 321);
        assert!(snapshot.recent_tasks.iter().any(|task| task.task.contains("ship auth")));
        assert!(snapshot.recent_tasks.iter().any(|task| task.status == "awaiting-approval"));
    }
}
