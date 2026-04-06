use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::dispatcher::Dispatcher;
use crate::error::CoreError;
use crate::events::AgentEvent;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunDetail {
    pub metadata: RunMetadata,
    pub entries: Vec<RunLogEntry>,
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RunStore {
    base_dir: Arc<PathBuf>,
}

impl RunStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: Arc::new(base_dir.into()),
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

        Ok(metadata)
    }

    pub fn append_event(&self, run_id: &str, event: &AgentEvent) -> Result<(), CoreError> {
        self.append_entry(
            run_id,
            "agent-event",
            serde_json::to_value(event)?,
        )
    }

    pub fn append_note(&self, run_id: &str, kind: &str, payload: Value) -> Result<(), CoreError> {
        self.append_entry(run_id, kind, payload)
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
        let mut metadata = self.load_metadata(run_id)?;
        metadata.finished_at = Some(Utc::now());
        metadata.status = status;
        metadata.output_preview = Some(truncate_preview(output_preview.as_ref()));
        self.write_metadata(&metadata)?;
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
        Ok(metadata)
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

    pub fn read_text_artifact(
        &self,
        run_id: &str,
        filename: &str,
    ) -> Result<String, CoreError> {
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

    pub fn list_artifacts(&self, run_id: &str) -> Result<Vec<String>, CoreError> {
        let run_dir = self.run_dir(run_id);
        if !run_dir.exists() {
            return Ok(Vec::new());
        }

        let mut artifacts = Vec::new();
        let entries = std::fs::read_dir(&run_dir).map_err(|error| CoreError::io(&run_dir, error))?;
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

fn truncate_preview(output: &str) -> String {
    const MAX_CHARS: usize = 240;
    let preview: String = output.chars().take(MAX_CHARS).collect();
    if output.chars().count() > MAX_CHARS {
        format!("{preview}...")
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::{Dispatcher, LocalDispatcher};
    use crate::types::PromptRef;

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
        assert!(detail.artifacts.is_empty());
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
        assert_eq!(artifacts, vec!["workflow-request.json"]);
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
        let run = store.create_run("task", "trace me", "standalone", Some("mock")).unwrap();
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
        let first = store.create_run("task", "first", "standalone", Some("codex")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = store.create_run("task", "second", "standalone", Some("codex")).unwrap();

        let runs = store.list_runs(10).unwrap();
        assert_eq!(runs[0].id, second.id);
        assert_eq!(runs[1].id, first.id);
    }
}
