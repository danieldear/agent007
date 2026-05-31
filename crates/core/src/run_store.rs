use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use tokio::task::JoinHandle;
use uuid::Uuid;

use agent007_models::CompletionResponse;

use crate::dispatcher::Dispatcher;
use crate::error::CoreError;
use crate::events::AgentEvent;
use crate::types::PromptRef;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RunStatus {
    Running,
    AwaitingApproval,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetadata {
    pub id: String,
    pub kind: String,
    pub task: String,
    pub mode: String,
    pub provider: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    pub output_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLogEntry {
    pub timestamp: DateTime<Utc>,
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentMessageKind {
    Note,
    Request,
    Handoff,
    Progress,
    Warning,
    Result,
}

impl AgentMessageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Request => "request",
            Self::Handoff => "handoff",
            Self::Progress => "progress",
            Self::Warning => "warning",
            Self::Result => "result",
        }
    }
}

impl std::str::FromStr for AgentMessageKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "note" => Ok(Self::Note),
            "request" => Ok(Self::Request),
            "handoff" => Ok(Self::Handoff),
            "progress" | "progress-note" => Ok(Self::Progress),
            "warning" | "blocker" => Ok(Self::Warning),
            "result" | "summary" | "result-summary" => Ok(Self::Result),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for AgentMessageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub run_id: String,
    pub session_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub from: String,
    pub to: Option<String>,
    pub kind: AgentMessageKind,
    pub body: String,
    #[serde(default)]
    pub payload: Value,
}

impl AgentMessage {
    pub fn new(
        run_id: impl Into<String>,
        from: impl Into<String>,
        to: Option<String>,
        kind: AgentMessageKind,
        body: impl Into<String>,
        payload: Value,
    ) -> Self {
        let run_id = run_id.into();
        Self {
            id: Uuid::new_v4().to_string(),
            session_id: Some(run_id.clone()),
            run_id,
            created_at: Utc::now(),
            from: from.into(),
            to,
            kind,
            body: body.into(),
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunDetail {
    pub metadata: RunMetadata,
    pub entries: Vec<RunLogEntry>,
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RunCostMode {
    #[default]
    FallbackEstimate,
    ProviderEstimate,
    HostReported,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RunTokenSummary {
    pub tokens: u64,
    pub requests: u32,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub estimated_usd: f64,
    #[serde(default)]
    pub cost_mode: RunCostMode,
}

const TOKEN_SUMMARY_ARTIFACT: &str = "token-summary.json";
const RUN_SCORECARD_ARTIFACT: &str = "run-scorecard.json";
const RUN_SCORECARD_SCHEMA_VERSION: u32 = 1;
// Keep in sync with STATUSLINE_PRICE_PER_TOKEN in crates/cli/src/commands/serve.rs.
pub const TOKEN_PRICE_PER_TOKEN_USD: f64 = 0.000_002;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunScorecard {
    pub schema_version: u32,
    pub run_id: String,
    pub kind: String,
    pub workflow: Option<String>,
    pub mode: String,
    pub provider: Option<String>,
    pub status: RunStatus,
    pub completed: bool,
    pub success: bool,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub tokens: u64,
    pub requests: u32,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub estimated_usd: f64,
    #[serde(default)]
    pub cost_mode: RunCostMode,
    pub retry_count: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
    pub quality_score: f64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
struct StoredWorkflowRequest {
    workflow: String,
}

#[derive(Debug, Clone)]
pub struct RunStore {
    base_dir: Arc<PathBuf>,
    token_summary_lock: Arc<Mutex<()>>,
    messages_lock: Arc<Mutex<()>>,
}

impl RunStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        let lock_key = std::fs::canonicalize(&base_dir).unwrap_or_else(|_| base_dir.clone());
        Self {
            base_dir: Arc::new(base_dir),
            token_summary_lock: shared_token_summary_lock(&lock_key),
            messages_lock: shared_messages_lock(&lock_key),
        }
    }

    pub fn create_run(
        &self,
        kind: &str,
        task: &str,
        mode: &str,
        provider: Option<&str>,
    ) -> Result<RunMetadata, CoreError> {
        self.ensure_base_dir()?;

        let metadata = RunMetadata {
            id: Uuid::new_v4().to_string(),
            kind: kind.to_string(),
            task: task.to_string(),
            mode: mode.to_string(),
            provider: provider.map(str::to_string),
            started_at: Utc::now(),
            finished_at: None,
            status: RunStatus::Running,
            output_preview: None,
        };

        let run_dir = self.run_dir(&metadata.id);
        std::fs::create_dir_all(&run_dir).map_err(|error| CoreError::io(&run_dir, error))?;
        self.write_metadata(&metadata)?;
        let _ = self.upsert_run_scorecard_for_metadata(&metadata);

        Ok(metadata)
    }

    pub fn append_event(&self, run_id: &str, event: &AgentEvent) -> Result<(), CoreError> {
        self.append_entry(run_id, "agent-event", serde_json::to_value(event)?)?;
        if let AgentEvent::ModelRequest { token_estimate, .. } = event {
            self.bump_token_summary(run_id, *token_estimate)?;
        }
        let _ = self.update_scorecard_from_event(run_id, event);
        Ok(())
    }

    pub fn append_note(&self, run_id: &str, kind: &str, payload: Value) -> Result<(), CoreError> {
        let scorecard_payload = payload.clone();
        self.append_entry(run_id, kind, payload)?;
        let _ = self.update_scorecard_from_note(run_id, kind, &scorecard_payload);
        Ok(())
    }

    pub fn append_message(&self, run_id: &str, message: AgentMessage) -> Result<(), CoreError> {
        if message.run_id != run_id {
            return Err(CoreError::DispatchFailed(format!(
                "message.run_id '{}' does not match run_id '{}'",
                message.run_id, run_id
            )));
        }
        let _guard = self
            .messages_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut messages = self
            .read_json_artifact_optional::<Vec<AgentMessage>>(run_id, "messages.json")?
            .unwrap_or_default();
        messages.push(message.clone());
        self.write_json_artifact(run_id, "messages.json", &messages)?;
        self.append_note(
            run_id,
            "agent-message",
            serde_json::json!({
                "id": message.id,
                "from": message.from,
                "to": message.to,
                "kind": message.kind,
                "body": message.body,
                "payload": message.payload,
            }),
        )
    }

    pub fn read_messages(&self, run_id: &str) -> Result<Vec<AgentMessage>, CoreError> {
        Ok(self
            .read_json_artifact_optional::<Vec<AgentMessage>>(run_id, "messages.json")?
            .unwrap_or_default())
    }

    pub fn finish_run(
        &self,
        run_id: &str,
        success: bool,
        output_preview: impl AsRef<str>,
    ) -> Result<RunMetadata, CoreError> {
        let status = if success {
            RunStatus::Succeeded
        } else {
            RunStatus::Failed
        };
        self.finish_run_with_status(run_id, status, output_preview)
    }

    pub fn finish_run_with_status(
        &self,
        run_id: &str,
        status: RunStatus,
        output_preview: impl AsRef<str>,
    ) -> Result<RunMetadata, CoreError> {
        let output_preview = output_preview.as_ref();
        let mut metadata = self.load_metadata(run_id)?;
        self.ensure_hosted_token_fallback(&metadata, output_preview)?;
        self.ensure_token_summary_artifact(run_id)?;
        metadata.finished_at = Some(Utc::now());
        metadata.status = status;
        metadata.output_preview = Some(truncate_preview(output_preview));
        self.write_metadata(&metadata)?;
        let _ = self.upsert_run_scorecard_for_metadata(&metadata);
        Ok(metadata)
    }

    pub fn update_run_status(
        &self,
        run_id: &str,
        status: RunStatus,
        output_preview: Option<String>,
    ) -> Result<RunMetadata, CoreError> {
        let mut metadata = self.load_metadata(run_id)?;
        metadata.status = status.clone();
        metadata.output_preview = output_preview.map(|value| truncate_preview(&value));
        metadata.finished_at = match status {
            RunStatus::Running | RunStatus::AwaitingApproval => None,
            RunStatus::Succeeded | RunStatus::Failed => Some(Utc::now()),
        };
        self.write_metadata(&metadata)?;
        let _ = self.upsert_run_scorecard_for_metadata(&metadata);
        Ok(metadata)
    }

    /// Update the provider field in a run's metadata so the dashboard shows
    /// the real model name (e.g. "claude-sonnet-4-6") instead of "hosted-mcp".
    /// Silently succeeds if the run does not exist.
    pub fn set_provider(&self, run_id: &str, provider: &str) -> Result<(), CoreError> {
        if let Ok(mut metadata) = self.load_metadata(run_id) {
            metadata.provider = Some(provider.to_string());
            self.write_metadata(&metadata)?;
            let _ = self.upsert_run_scorecard_for_metadata(&metadata);
        }
        Ok(())
    }

    /// On server startup, mark any runs that are still in `Running` or
    /// `AwaitingApproval` state as `Failed` with `finished_at = now`.
    /// This prevents stale runs (left open by a crash or SIGKILL) from
    /// permanently showing `finished_at: null` in the dashboard and
    /// blocking the NightlyLearner / feedback collector from closing.
    /// Returns the number of runs that were cleaned up.
    pub fn cleanup_stale_runs(&self) -> usize {
        self.ensure_base_dir().ok();
        let entries = match std::fs::read_dir(self.base_dir.as_ref()) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        let mut cleaned = 0usize;
        for entry in entries.flatten() {
            let meta_path = entry.path().join("meta.json");
            if !meta_path.exists() {
                continue;
            }
            let raw = match std::fs::read_to_string(&meta_path) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let mut metadata: RunMetadata = match serde_json::from_str(&raw) {
                Ok(m) => m,
                Err(_) => continue,
            };
            // Only fail truly stale Running runs. AwaitingApproval runs with
            // finished_at=None come from the hosted workflow path (update_run_status)
            // and represent legitimate human-in-the-loop pauses — failing them on
            // restart is incorrect. They persist so the user can resume or cancel manually.
            if matches!(metadata.status, RunStatus::Running) && metadata.finished_at.is_none() {
                metadata.status = RunStatus::Failed;
                metadata.finished_at = Some(Utc::now());
                let preview = "terminated: server restarted";
                metadata.output_preview = Some(preview.to_string());
                if let Ok(json) = serde_json::to_string_pretty(&metadata) {
                    let _ = std::fs::write(&meta_path, json);
                    // Synthesize a token fallback and summary so orphaned hosted-mcp runs
                    // don't show 0 tokens / 0 requests in the dashboard after cleanup.
                    let _ = self.ensure_hosted_token_fallback(&metadata, preview);
                    let _ = self.ensure_token_summary_artifact(&metadata.id);
                    let _ = self.upsert_run_scorecard_for_metadata(&metadata);
                    cleaned += 1;
                }
            }
        }
        cleaned
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<RunMetadata>, CoreError> {
        self.ensure_base_dir()?;
        let mut runs = Vec::new();
        let entries = std::fs::read_dir(self.base_dir.as_ref())
            .map_err(|error| CoreError::io(self.base_dir.as_ref(), error))?;

        for entry in entries {
            let entry = entry.map_err(|error| CoreError::io(self.base_dir.as_ref(), error))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let meta_path = path.join("meta.json");
            if !meta_path.exists() {
                continue;
            }
            let raw = std::fs::read_to_string(&meta_path)
                .map_err(|error| CoreError::io(&meta_path, error))?;
            let metadata: RunMetadata = serde_json::from_str(&raw)?;
            runs.push(metadata);
        }

        runs.sort_by(|left, right| right.started_at.cmp(&left.started_at));
        runs.truncate(limit);
        Ok(runs)
    }

    pub fn load_run(&self, run_id: &str) -> Result<RunDetail, CoreError> {
        let metadata = self.load_metadata(run_id)?;
        let mut entries = Vec::new();
        let log_path = self.events_path(run_id);
        if log_path.exists() {
            let raw = std::fs::read_to_string(&log_path)
                .map_err(|error| CoreError::io(&log_path, error))?;
            for line in raw.lines().filter(|line| !line.trim().is_empty()) {
                entries.push(serde_json::from_str::<RunLogEntry>(line)?);
            }
        }

        let artifacts = self.list_artifacts(run_id)?;

        Ok(RunDetail {
            metadata,
            entries,
            artifacts,
        })
    }

    pub fn write_json_artifact<T: Serialize>(
        &self,
        run_id: &str,
        filename: &str,
        value: &T,
    ) -> Result<(), CoreError> {
        let artifact_path = self.artifact_path(run_id, filename);
        if let Some(parent) = artifact_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| CoreError::io(parent, error))?;
        }
        let raw = serde_json::to_string_pretty(value)?;
        std::fs::write(&artifact_path, raw).map_err(|error| CoreError::io(&artifact_path, error))
    }

    pub fn write_text_artifact(
        &self,
        run_id: &str,
        filename: &str,
        value: impl AsRef<str>,
    ) -> Result<(), CoreError> {
        let artifact_path = self.artifact_path(run_id, filename);
        if let Some(parent) = artifact_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| CoreError::io(parent, error))?;
        }
        std::fs::write(&artifact_path, value.as_ref())
            .map_err(|error| CoreError::io(&artifact_path, error))
    }

    pub fn read_json_artifact<T: DeserializeOwned>(
        &self,
        run_id: &str,
        filename: &str,
    ) -> Result<T, CoreError> {
        let artifact_path = self.artifact_path(run_id, filename);
        let raw = std::fs::read_to_string(&artifact_path)
            .map_err(|error| CoreError::io(&artifact_path, error))?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn read_json_artifact_optional<T: DeserializeOwned>(
        &self,
        run_id: &str,
        filename: &str,
    ) -> Result<Option<T>, CoreError> {
        let artifact_path = self.artifact_path(run_id, filename);
        if !artifact_path.exists() {
            return Ok(None);
        }
        self.read_json_artifact(run_id, filename).map(Some)
    }

    pub fn read_text_artifact(&self, run_id: &str, filename: &str) -> Result<String, CoreError> {
        let artifact_path = self.artifact_path(run_id, filename);
        std::fs::read_to_string(&artifact_path)
            .map_err(|error| CoreError::io(&artifact_path, error))
    }

    pub fn read_text_artifact_optional(
        &self,
        run_id: &str,
        filename: &str,
    ) -> Result<Option<String>, CoreError> {
        let artifact_path = self.artifact_path(run_id, filename);
        if !artifact_path.exists() {
            return Ok(None);
        }
        self.read_text_artifact(run_id, filename).map(Some)
    }

    pub fn read_run_scorecard(&self, run_id: &str) -> Result<RunScorecard, CoreError> {
        self.read_json_artifact(run_id, RUN_SCORECARD_ARTIFACT)
    }

    pub fn read_run_scorecard_optional(
        &self,
        run_id: &str,
    ) -> Result<Option<RunScorecard>, CoreError> {
        self.read_json_artifact_optional(run_id, RUN_SCORECARD_ARTIFACT)
    }

    pub fn ensure_run_scorecard_artifact(&self, run_id: &str) -> Result<RunScorecard, CoreError> {
        let metadata = self.load_metadata(run_id)?;
        self.upsert_run_scorecard_for_metadata(&metadata)?;
        self.read_run_scorecard(run_id)
    }

    pub fn recent_scorecards_for_workflow(
        &self,
        workflow: &str,
        exclude_run_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RunScorecard>, CoreError> {
        let mut scorecards = Vec::new();
        let scan_limit = limit.max(25).saturating_mul(10);
        for run in self.list_runs(scan_limit)? {
            if exclude_run_id
                .map(|excluded| excluded == run.id.as_str())
                .unwrap_or(false)
            {
                continue;
            }
            let Ok(scorecard) = self.ensure_run_scorecard_artifact(&run.id) else {
                continue;
            };
            if !scorecard.completed {
                continue;
            }
            if scorecard.workflow.as_deref() != Some(workflow) {
                continue;
            }
            scorecards.push(scorecard);
            if scorecards.len() >= limit {
                break;
            }
        }
        Ok(scorecards)
    }

    pub fn list_artifacts(&self, run_id: &str) -> Result<Vec<String>, CoreError> {
        let run_dir = self.run_dir(run_id);
        if !run_dir.exists() {
            return Ok(Vec::new());
        }

        let mut artifacts = Vec::new();
        let entries =
            std::fs::read_dir(&run_dir).map_err(|error| CoreError::io(&run_dir, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| CoreError::io(&run_dir, error))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if matches!(name, "meta.json" | "events.jsonl") {
                continue;
            }
            artifacts.push(name.to_string());
        }
        artifacts.sort();
        Ok(artifacts)
    }

    pub async fn spawn_dispatcher_trace(
        &self,
        run_id: String,
        dispatcher: Arc<dyn Dispatcher>,
    ) -> Result<JoinHandle<()>, CoreError> {
        let mut events = dispatcher.subscribe().await?;
        let store = self.clone();
        Ok(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let _ = store.append_event(&run_id, &event);
                if matches!(event, AgentEvent::TaskCompleted { .. }) {
                    break;
                }
            }
        }))
    }

    fn ensure_hosted_token_fallback(
        &self,
        metadata: &RunMetadata,
        output_preview: &str,
    ) -> Result<(), CoreError> {
        if metadata.mode != "hosted-mcp" || output_preview.trim().is_empty() {
            return Ok(());
        }
        if self.has_model_request_event(&metadata.id)? {
            return Ok(());
        }
        let token_estimate = (output_preview.chars().count() / 4).max(1);
        let provider = metadata
            .provider
            .clone()
            .unwrap_or_else(|| metadata.mode.clone());
        self.append_event(
            &metadata.id,
            &AgentEvent::ModelRequest {
                provider,
                prompt_ref: PromptRef::new(),
                token_estimate,
            },
        )
    }

    pub fn has_model_request_event(&self, run_id: &str) -> Result<bool, CoreError> {
        let log_path = self.events_path(run_id);
        if !log_path.exists() {
            return Ok(false);
        }
        let file =
            std::fs::File::open(&log_path).map_err(|error| CoreError::io(&log_path, error))?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line.map_err(|error| CoreError::io(&log_path, error))?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: RunLogEntry = serde_json::from_str(&line)?;
            if entry.kind != "agent-event" {
                continue;
            }
            if let Ok(AgentEvent::ModelRequest { .. }) =
                serde_json::from_value::<AgentEvent>(entry.payload)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn bump_token_summary(&self, run_id: &str, token_estimate: usize) -> Result<(), CoreError> {
        let _guard = self
            .token_summary_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut summary = self
            .read_json_artifact_optional::<RunTokenSummary>(run_id, TOKEN_SUMMARY_ARTIFACT)?
            .unwrap_or_default();
        summary.tokens += token_estimate as u64;
        summary.requests += 1;
        if summary.cost_mode == RunCostMode::FallbackEstimate {
            summary.estimated_usd = summary.tokens as f64 * TOKEN_PRICE_PER_TOKEN_USD;
        } else {
            summary.estimated_usd += token_estimate as f64 * TOKEN_PRICE_PER_TOKEN_USD;
        }
        self.write_json_artifact(run_id, TOKEN_SUMMARY_ARTIFACT, &summary)
    }

    pub fn overwrite_token_summary(
        &self,
        run_id: &str,
        summary: &RunTokenSummary,
    ) -> Result<(), CoreError> {
        let _guard = self
            .token_summary_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.write_json_artifact(run_id, TOKEN_SUMMARY_ARTIFACT, summary)
    }

    pub fn add_token_summary(
        &self,
        run_id: &str,
        delta: &RunTokenSummary,
    ) -> Result<(), CoreError> {
        let _guard = self
            .token_summary_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut summary = self
            .read_json_artifact_optional::<RunTokenSummary>(run_id, TOKEN_SUMMARY_ARTIFACT)?
            .unwrap_or_default();
        summary.tokens = summary.tokens.saturating_add(delta.tokens);
        summary.requests = summary.requests.saturating_add(delta.requests);
        summary.input_tokens = summary.input_tokens.saturating_add(delta.input_tokens);
        summary.output_tokens = summary.output_tokens.saturating_add(delta.output_tokens);
        summary.cache_read_tokens = summary
            .cache_read_tokens
            .saturating_add(delta.cache_read_tokens);
        summary.cache_write_tokens = summary
            .cache_write_tokens
            .saturating_add(delta.cache_write_tokens);
        summary.estimated_usd += delta.estimated_usd;
        summary.cost_mode = match (&summary.cost_mode, &delta.cost_mode) {
            (_, RunCostMode::ProviderEstimate) => RunCostMode::ProviderEstimate,
            (_, RunCostMode::HostReported) => RunCostMode::HostReported,
            (RunCostMode::ProviderEstimate, _) => RunCostMode::ProviderEstimate,
            (RunCostMode::HostReported, _) => RunCostMode::HostReported,
            _ => RunCostMode::FallbackEstimate,
        };
        self.write_json_artifact(run_id, TOKEN_SUMMARY_ARTIFACT, &summary)
    }

    fn ensure_token_summary_artifact(&self, run_id: &str) -> Result<(), CoreError> {
        let _guard = self
            .token_summary_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self
            .read_json_artifact_optional::<RunTokenSummary>(run_id, TOKEN_SUMMARY_ARTIFACT)?
            .is_some()
        {
            return Ok(());
        }
        let summary = self.compute_token_summary(run_id)?;
        self.write_json_artifact(run_id, TOKEN_SUMMARY_ARTIFACT, &summary)
    }

    fn update_scorecard_from_event(
        &self,
        run_id: &str,
        event: &AgentEvent,
    ) -> Result<(), CoreError> {
        let metadata = self.load_metadata(run_id)?;
        let summary = self
            .read_json_artifact_optional::<RunTokenSummary>(run_id, TOKEN_SUMMARY_ARTIFACT)?
            .unwrap_or_default();
        let mut scorecard = self.load_or_init_scorecard(&metadata, &summary)?;
        match event {
            AgentEvent::ToolCall { .. } => {
                scorecard.tool_calls = scorecard.tool_calls.saturating_add(1);
            }
            AgentEvent::ToolCallResult { success, .. } => {
                if !success {
                    scorecard.tool_errors = scorecard.tool_errors.saturating_add(1);
                }
            }
            _ => {}
        }
        self.sync_scorecard_with_metadata(&mut scorecard, &metadata, &summary);
        self.write_json_artifact(run_id, RUN_SCORECARD_ARTIFACT, &scorecard)
    }

    fn update_scorecard_from_note(
        &self,
        run_id: &str,
        kind: &str,
        _payload: &Value,
    ) -> Result<(), CoreError> {
        if kind != "workflow-step-retry" {
            return Ok(());
        }
        let metadata = self.load_metadata(run_id)?;
        let summary = self
            .read_json_artifact_optional::<RunTokenSummary>(run_id, TOKEN_SUMMARY_ARTIFACT)?
            .unwrap_or_default();
        let mut scorecard = self.load_or_init_scorecard(&metadata, &summary)?;
        scorecard.retry_count = scorecard.retry_count.saturating_add(1);
        self.sync_scorecard_with_metadata(&mut scorecard, &metadata, &summary);
        self.write_json_artifact(run_id, RUN_SCORECARD_ARTIFACT, &scorecard)
    }

    fn upsert_run_scorecard_for_metadata(&self, metadata: &RunMetadata) -> Result<(), CoreError> {
        let summary = self
            .read_json_artifact_optional::<RunTokenSummary>(&metadata.id, TOKEN_SUMMARY_ARTIFACT)?
            .unwrap_or_default();
        let mut scorecard = self.load_or_init_scorecard(metadata, &summary)?;
        self.sync_scorecard_with_metadata(&mut scorecard, metadata, &summary);
        self.write_json_artifact(&metadata.id, RUN_SCORECARD_ARTIFACT, &scorecard)
    }

    fn load_or_init_scorecard(
        &self,
        metadata: &RunMetadata,
        summary: &RunTokenSummary,
    ) -> Result<RunScorecard, CoreError> {
        if let Some(scorecard) =
            self.read_json_artifact_optional::<RunScorecard>(&metadata.id, RUN_SCORECARD_ARTIFACT)?
        {
            return Ok(scorecard);
        }

        let (retry_count, tool_calls, tool_errors) = self.scan_run_activity(&metadata.id)?;
        let completed = run_is_completed(&metadata.status);
        let success = matches!(metadata.status, RunStatus::Succeeded);
        let duration_ms = run_duration_ms(metadata.started_at, metadata.finished_at);
        let mut scorecard = RunScorecard {
            schema_version: RUN_SCORECARD_SCHEMA_VERSION,
            run_id: metadata.id.clone(),
            kind: metadata.kind.clone(),
            workflow: self.resolve_workflow_name(&metadata.id),
            mode: metadata.mode.clone(),
            provider: metadata.provider.clone(),
            status: metadata.status.clone(),
            completed,
            success,
            started_at: metadata.started_at,
            finished_at: metadata.finished_at,
            duration_ms,
            tokens: summary.tokens,
            requests: summary.requests,
            estimated_usd: summary.tokens as f64 * TOKEN_PRICE_PER_TOKEN_USD,
            input_tokens: summary.input_tokens,
            output_tokens: summary.output_tokens,
            cache_read_tokens: summary.cache_read_tokens,
            cache_write_tokens: summary.cache_write_tokens,
            cost_mode: summary.cost_mode.clone(),
            retry_count,
            tool_calls,
            tool_errors,
            quality_score: 0.0,
            updated_at: Utc::now(),
        };
        scorecard.quality_score = compute_quality_score(&scorecard);
        Ok(scorecard)
    }

    fn sync_scorecard_with_metadata(
        &self,
        scorecard: &mut RunScorecard,
        metadata: &RunMetadata,
        summary: &RunTokenSummary,
    ) {
        scorecard.schema_version = RUN_SCORECARD_SCHEMA_VERSION;
        scorecard.run_id = metadata.id.clone();
        scorecard.kind = metadata.kind.clone();
        scorecard.workflow = self.resolve_workflow_name(&metadata.id);
        scorecard.mode = metadata.mode.clone();
        scorecard.provider = metadata.provider.clone();
        scorecard.status = metadata.status.clone();
        scorecard.completed = run_is_completed(&metadata.status);
        scorecard.success = matches!(metadata.status, RunStatus::Succeeded);
        scorecard.started_at = metadata.started_at;
        scorecard.finished_at = metadata.finished_at;
        scorecard.duration_ms = run_duration_ms(metadata.started_at, metadata.finished_at);
        scorecard.tokens = summary.tokens;
        scorecard.requests = summary.requests;
        scorecard.input_tokens = summary.input_tokens;
        scorecard.output_tokens = summary.output_tokens;
        scorecard.cache_read_tokens = summary.cache_read_tokens;
        scorecard.cache_write_tokens = summary.cache_write_tokens;
        scorecard.estimated_usd = if summary.estimated_usd > 0.0 {
            summary.estimated_usd
        } else {
            summary.tokens as f64 * TOKEN_PRICE_PER_TOKEN_USD
        };
        scorecard.cost_mode = summary.cost_mode.clone();
        scorecard.quality_score = compute_quality_score(scorecard);
        scorecard.updated_at = Utc::now();
    }

    fn resolve_workflow_name(&self, run_id: &str) -> Option<String> {
        self.read_json_artifact_optional::<StoredWorkflowRequest>(run_id, "workflow-request.json")
            .ok()
            .flatten()
            .map(|request| request.workflow)
    }

    fn scan_run_activity(&self, run_id: &str) -> Result<(u32, u32, u32), CoreError> {
        let log_path = self.events_path(run_id);
        if !log_path.exists() {
            return Ok((0, 0, 0));
        }

        let file =
            std::fs::File::open(&log_path).map_err(|error| CoreError::io(&log_path, error))?;
        let reader = BufReader::new(file);

        let mut retry_count = 0u32;
        let mut tool_calls = 0u32;
        let mut tool_errors = 0u32;

        for line in reader.lines() {
            let line = line.map_err(|error| CoreError::io(&log_path, error))?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: RunLogEntry = serde_json::from_str(&line)?;
            match entry.kind.as_str() {
                "workflow-step-retry" => {
                    retry_count = retry_count.saturating_add(1);
                }
                "agent-event" => {
                    if let Ok(event) = serde_json::from_value::<AgentEvent>(entry.payload) {
                        match event {
                            AgentEvent::ToolCall { .. } => {
                                tool_calls = tool_calls.saturating_add(1);
                            }
                            AgentEvent::ToolCallResult { success, .. } => {
                                if !success {
                                    tool_errors = tool_errors.saturating_add(1);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        Ok((retry_count, tool_calls, tool_errors))
    }

    fn compute_token_summary(&self, run_id: &str) -> Result<RunTokenSummary, CoreError> {
        let log_path = self.events_path(run_id);
        if !log_path.exists() {
            return Ok(RunTokenSummary::default());
        }
        let file =
            std::fs::File::open(&log_path).map_err(|error| CoreError::io(&log_path, error))?;
        let reader = BufReader::new(file);
        let mut summary = RunTokenSummary::default();
        for line in reader.lines() {
            let line = line.map_err(|error| CoreError::io(&log_path, error))?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: RunLogEntry = serde_json::from_str(&line)?;
            if entry.kind != "agent-event" {
                continue;
            }
            if let Ok(AgentEvent::ModelRequest { token_estimate, .. }) =
                serde_json::from_value::<AgentEvent>(entry.payload)
            {
                summary.tokens += token_estimate as u64;
                summary.requests += 1;
            }
        }
        summary.estimated_usd = summary.tokens as f64 * TOKEN_PRICE_PER_TOKEN_USD;
        summary.cost_mode = RunCostMode::FallbackEstimate;
        Ok(summary)
    }

    fn ensure_base_dir(&self) -> Result<(), CoreError> {
        std::fs::create_dir_all(self.base_dir.as_ref())
            .map_err(|error| CoreError::io(self.base_dir.as_ref(), error))
    }

    fn run_dir(&self, run_id: &str) -> PathBuf {
        self.base_dir.join(run_id)
    }

    fn meta_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("meta.json")
    }

    fn events_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("events.jsonl")
    }

    fn artifact_path(&self, run_id: &str, filename: &str) -> PathBuf {
        self.run_dir(run_id).join(filename)
    }

    fn write_metadata(&self, metadata: &RunMetadata) -> Result<(), CoreError> {
        let meta_path = self.meta_path(&metadata.id);
        if let Some(parent) = meta_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| CoreError::io(parent, error))?;
        }
        let raw = serde_json::to_string_pretty(metadata)?;
        std::fs::write(&meta_path, raw).map_err(|error| CoreError::io(&meta_path, error))
    }

    fn load_metadata(&self, run_id: &str) -> Result<RunMetadata, CoreError> {
        let meta_path = self.meta_path(run_id);
        let raw = std::fs::read_to_string(&meta_path)
            .map_err(|error| CoreError::io(&meta_path, error))?;
        Ok(serde_json::from_str(&raw)?)
    }

    fn append_entry(&self, run_id: &str, kind: &str, payload: Value) -> Result<(), CoreError> {
        let log_path = self.events_path(run_id);
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| CoreError::io(parent, error))?;
        }

        let entry = RunLogEntry {
            timestamp: Utc::now(),
            kind: kind.to_string(),
            payload,
        };
        let encoded = serde_json::to_string(&entry)?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|error| CoreError::io(&log_path, error))?;
        writeln!(file, "{encoded}").map_err(|error| CoreError::io(&log_path, error))
    }
}

impl RunTokenSummary {
    pub fn from_completion_response(
        response: &CompletionResponse,
        fallback_tokens: Option<u64>,
    ) -> Option<Self> {
        let tokens = response.total_tokens_with_fallback().or(fallback_tokens)?;
        let input_tokens = response.input_tokens.unwrap_or(0) as u64;
        let output_tokens = response.output_tokens.unwrap_or(0) as u64;
        let cache_read_tokens = response.cached_tokens.unwrap_or(0) as u64;
        let cache_write_tokens = response.cache_write_tokens.unwrap_or(0) as u64;
        let cost_mode = if response.estimated_cost_usd.is_some() {
            RunCostMode::ProviderEstimate
        } else {
            RunCostMode::FallbackEstimate
        };
        let estimated_usd = response
            .estimated_cost_usd
            .unwrap_or(tokens as f64 * TOKEN_PRICE_PER_TOKEN_USD);

        Some(Self {
            tokens,
            requests: 1,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            estimated_usd,
            cost_mode,
        })
    }
}

fn truncate_preview(output: &str) -> String {
    const MAX_CHARS: usize = 240;
    let preview: String = output.chars().take(MAX_CHARS).collect();
    if output.chars().count() > MAX_CHARS {
        format!("{preview}...")
    } else {
        preview
    }
}

fn run_duration_ms(started_at: DateTime<Utc>, finished_at: Option<DateTime<Utc>>) -> Option<i64> {
    finished_at.map(|finished| {
        finished
            .signed_duration_since(started_at)
            .num_milliseconds()
            .max(0)
    })
}

fn run_is_completed(status: &RunStatus) -> bool {
    matches!(status, RunStatus::Succeeded | RunStatus::Failed)
}

fn compute_quality_score(scorecard: &RunScorecard) -> f64 {
    if !scorecard.completed {
        return 0.0;
    }

    let mut score = if scorecard.success { 100.0 } else { 0.0 };
    score -= scorecard.retry_count as f64 * 4.0;
    score -= scorecard.tool_errors as f64 * 6.0;
    score -= (scorecard.tokens as f64 / 100_000.0).min(15.0);
    if let Some(duration_ms) = scorecard.duration_ms {
        score -= (duration_ms as f64 / 1_000.0 / 60.0).min(10.0);
    }
    (score.clamp(0.0, 100.0) * 10.0).round() / 10.0
}

fn shared_token_summary_lock(base_dir: &PathBuf) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<std::collections::HashMap<PathBuf, Arc<Mutex<()>>>>> =
        OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut guard = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .entry(base_dir.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn shared_messages_lock(base_dir: &PathBuf) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<std::collections::HashMap<PathBuf, Arc<Mutex<()>>>>> =
        OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut guard = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .entry(base_dir.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::{Dispatcher, LocalDispatcher};

    #[test]
    fn create_and_finish_run_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        let run = store
            .create_run("task", "analyze repo", "standalone", Some("codex"))
            .unwrap();
        assert_eq!(run.status, RunStatus::Running);

        let finished = store.finish_run(&run.id, true, "done").unwrap();
        assert_eq!(finished.status, RunStatus::Succeeded);
        assert_eq!(finished.output_preview.as_deref(), Some("done"));
    }

    #[test]
    fn load_run_includes_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store.create_run("task", "hello", "delegate", None).unwrap();
        store
            .append_note(&run.id, "note", serde_json::json!({ "value": 1 }))
            .unwrap();

        let detail = store.load_run(&run.id).unwrap();
        assert_eq!(detail.entries.len(), 1);
        assert_eq!(detail.entries[0].kind, "note");
        assert!(detail
            .artifacts
            .contains(&RUN_SCORECARD_ARTIFACT.to_string()));
    }

    #[test]
    fn artifact_round_trip_works() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("workflow", "resume me", "standalone", Some("codex"))
            .unwrap();

        let payload = serde_json::json!({
            "workflow": "code-review",
            "task": "review auth module"
        });
        store
            .write_json_artifact(&run.id, "workflow-request.json", &payload)
            .unwrap();

        let loaded: serde_json::Value = store
            .read_json_artifact(&run.id, "workflow-request.json")
            .unwrap();
        assert_eq!(loaded, payload);

        let artifacts = store.list_artifacts(&run.id).unwrap();
        assert_eq!(
            artifacts,
            vec![
                RUN_SCORECARD_ARTIFACT.to_string(),
                "workflow-request.json".to_string()
            ]
        );
    }

    #[test]
    fn agent_messages_round_trip_and_append_to_log() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("workflow", "coordinate agents", "hosted-mcp", None)
            .unwrap();

        let message = AgentMessage::new(
            &run.id,
            "Planner",
            Some("Coder".to_string()),
            AgentMessageKind::Handoff,
            "Implement the selected plan.",
            serde_json::json!({ "step": "implement" }),
        );
        store.append_message(&run.id, message.clone()).unwrap();

        let messages = store.read_messages(&run.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].from, "Planner");
        assert_eq!(messages[0].to.as_deref(), Some("Coder"));
        assert_eq!(messages[0].kind, AgentMessageKind::Handoff);

        let detail = store.load_run(&run.id).unwrap();
        assert!(detail
            .entries
            .iter()
            .any(|entry| entry.kind == "agent-message"));
    }

    #[test]
    fn text_artifact_round_trip_works() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("compact", "summarize test output", "hosted-mcp", None)
            .unwrap();

        store
            .write_text_artifact(&run.id, "summary.txt", "hello compact world")
            .unwrap();

        let loaded = store.read_text_artifact(&run.id, "summary.txt").unwrap();
        assert_eq!(loaded, "hello compact world");

        let loaded_optional = store
            .read_text_artifact_optional(&run.id, "summary.txt")
            .unwrap();
        assert_eq!(loaded_optional.as_deref(), Some("hello compact world"));
    }

    #[test]
    fn update_run_status_keeps_active_runs_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("workflow", "ship feature", "hosted-mcp", None)
            .unwrap();

        let updated = store
            .update_run_status(
                &run.id,
                RunStatus::AwaitingApproval,
                Some("approval required".to_string()),
            )
            .unwrap();
        assert_eq!(updated.status, RunStatus::AwaitingApproval);
        assert!(updated.finished_at.is_none());
        assert_eq!(updated.output_preview.as_deref(), Some("approval required"));
    }

    #[tokio::test]
    async fn dispatcher_trace_persists_agent_events() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("task", "trace me", "standalone", Some("mock"))
            .unwrap();
        let dispatcher = LocalDispatcher::new(8);
        let trace = store
            .spawn_dispatcher_trace(
                run.id.clone(),
                dispatcher.clone() as Arc<dyn crate::dispatcher::Dispatcher>,
            )
            .await
            .unwrap();

        dispatcher
            .publish(AgentEvent::ModelRequest {
                provider: "mock".to_string(),
                prompt_ref: PromptRef::new(),
                token_estimate: 4,
            })
            .await
            .unwrap();
        dispatcher
            .publish(AgentEvent::TaskCompleted {
                agent_id: crate::types::AgentId::new(),
                result: crate::task::TaskResult::success(Uuid::new_v4(), "done".to_string()),
                skill_name: None,
                model: None,
            })
            .await
            .unwrap();

        trace.await.unwrap();

        let detail = store.load_run(&run.id).unwrap();
        assert_eq!(detail.entries.len(), 2);
        assert_eq!(detail.entries[0].kind, "agent-event");
    }

    #[test]
    fn list_runs_returns_latest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let first = store
            .create_run("task", "first", "standalone", Some("codex"))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = store
            .create_run("task", "second", "standalone", Some("codex"))
            .unwrap();

        let runs = store.list_runs(10).unwrap();
        assert_eq!(runs[0].id, second.id);
        assert_eq!(runs[1].id, first.id);
    }

    #[test]
    fn cleanup_stale_runs_marks_open_runs_failed() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        // Create two runs: one that finishes normally, one left open (simulates crash)
        let finished = store
            .create_run("task", "done", "standalone", None)
            .unwrap();
        store.finish_run(&finished.id, true, "ok").unwrap();

        let stale = store
            .create_run("task", "stale", "standalone", None)
            .unwrap();
        assert_eq!(stale.status, RunStatus::Running);
        assert!(stale.finished_at.is_none());

        let cleaned = store.cleanup_stale_runs();
        assert_eq!(cleaned, 1);

        let updated = store.load_run(&stale.id).unwrap().metadata;
        assert_eq!(updated.status, RunStatus::Failed);
        assert!(updated.finished_at.is_some());
        assert_eq!(
            updated.output_preview.as_deref(),
            Some("terminated: server restarted")
        );

        // The already-finished run should not be touched
        let still_ok = store.load_run(&finished.id).unwrap().metadata;
        assert_eq!(still_ok.status, RunStatus::Succeeded);
    }

    #[test]
    fn append_event_updates_token_summary_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("task", "track tokens", "standalone", Some("mock"))
            .unwrap();

        store
            .append_event(
                &run.id,
                &AgentEvent::ModelRequest {
                    provider: "mock".to_string(),
                    prompt_ref: PromptRef::new(),
                    token_estimate: 42,
                },
            )
            .unwrap();

        let summary: RunTokenSummary = store
            .read_json_artifact(&run.id, TOKEN_SUMMARY_ARTIFACT)
            .unwrap();
        assert_eq!(summary.tokens, 42);
        assert_eq!(summary.requests, 1);
    }

    #[test]
    fn append_event_preserves_host_reported_cost_mode() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("task", "preserve exact cost", "hosted-mcp", Some("claude"))
            .unwrap();

        store
            .overwrite_token_summary(
                &run.id,
                &RunTokenSummary {
                    tokens: 300,
                    requests: 1,
                    input_tokens: 100,
                    output_tokens: 80,
                    cache_read_tokens: 70,
                    cache_write_tokens: 50,
                    estimated_usd: 0.0125,
                    cost_mode: RunCostMode::HostReported,
                },
            )
            .unwrap();

        store
            .append_event(
                &run.id,
                &AgentEvent::ModelRequest {
                    provider: "claude".to_string(),
                    prompt_ref: PromptRef::new(),
                    token_estimate: 25,
                },
            )
            .unwrap();

        let summary: RunTokenSummary = store
            .read_json_artifact(&run.id, TOKEN_SUMMARY_ARTIFACT)
            .unwrap();
        assert_eq!(summary.tokens, 325);
        assert_eq!(summary.requests, 2);
        assert_eq!(summary.cost_mode, RunCostMode::HostReported);
        assert!(summary.estimated_usd >= 0.0125);
    }

    #[test]
    fn create_run_initializes_run_scorecard_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("task", "score me", "standalone", Some("mock"))
            .unwrap();

        let scorecard = store.read_run_scorecard(&run.id).unwrap();
        assert_eq!(scorecard.run_id, run.id);
        assert_eq!(scorecard.status, RunStatus::Running);
        assert_eq!(scorecard.tokens, 0);
        assert_eq!(scorecard.retry_count, 0);
        assert_eq!(scorecard.tool_errors, 0);
    }

    #[test]
    fn scorecard_tracks_retry_and_tool_failures() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run(
                "workflow",
                "retry and tool failure",
                "standalone",
                Some("mock"),
            )
            .unwrap();

        store
            .append_note(
                &run.id,
                "workflow-step-retry",
                serde_json::json!({ "attempt": 1 }),
            )
            .unwrap();
        store
            .append_event(
                &run.id,
                &AgentEvent::ToolCall {
                    agent_id: crate::types::AgentId::new(),
                    tool: crate::events::ToolCall {
                        name: "exec_command".to_string(),
                        args: serde_json::json!({ "cmd": "echo hi" }),
                    },
                },
            )
            .unwrap();
        store
            .append_event(
                &run.id,
                &AgentEvent::ToolCallResult {
                    agent_id: crate::types::AgentId::new(),
                    tool: crate::events::ToolCall {
                        name: "exec_command".to_string(),
                        args: serde_json::json!({ "cmd": "echo hi" }),
                    },
                    success: false,
                    error: Some("non-zero exit".to_string()),
                    duration_ms: None,
                },
            )
            .unwrap();
        store.finish_run(&run.id, false, "failed").unwrap();

        let scorecard = store.read_run_scorecard(&run.id).unwrap();
        assert_eq!(scorecard.retry_count, 1);
        assert_eq!(scorecard.tool_calls, 1);
        assert_eq!(scorecard.tool_errors, 1);
        assert_eq!(scorecard.status, RunStatus::Failed);
        assert!(scorecard.completed);
        assert!(!scorecard.success);
    }

    #[test]
    fn finish_run_adds_hosted_token_fallback_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let run = store
            .create_run("workflow", "hosted", "hosted-mcp", None)
            .unwrap();

        store
            .finish_run(&run.id, true, "hosted workflow output")
            .unwrap();

        let summary: RunTokenSummary = store
            .read_json_artifact(&run.id, TOKEN_SUMMARY_ARTIFACT)
            .unwrap();
        assert_eq!(summary.requests, 1);
        assert!(summary.tokens > 0);
    }

    #[test]
    fn token_summary_lock_is_shared_across_store_instances() {
        let dir = tempfile::tempdir().unwrap();
        let store_a = RunStore::new(dir.path());
        let run = store_a
            .create_run("task", "concurrent tokens", "hosted-mcp", None)
            .unwrap();
        let run_id = run.id.clone();
        let store_b = RunStore::new(dir.path());

        let threads: Vec<_> = (0..2)
            .map(|idx| {
                let run_id = run_id.clone();
                let store = if idx % 2 == 0 {
                    store_a.clone()
                } else {
                    store_b.clone()
                };
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        store
                            .append_event(
                                &run_id,
                                &AgentEvent::ModelRequest {
                                    provider: "hosted-mcp".to_string(),
                                    prompt_ref: PromptRef::new(),
                                    token_estimate: 1,
                                },
                            )
                            .unwrap();
                    }
                })
            })
            .collect();

        for thread in threads {
            thread.join().unwrap();
        }

        let summary: RunTokenSummary = store_a
            .read_json_artifact(&run_id, TOKEN_SUMMARY_ARTIFACT)
            .unwrap();
        assert_eq!(summary.tokens, 200);
        assert_eq!(summary.requests, 200);
    }
}
