use std::sync::Arc;
use futures::StreamExt as _;
use agent007_core::dispatcher::Dispatcher;
use agent007_core::events::AgentEvent;
use agent007_core::types::PromptRef;

pub struct FeedbackCollector {
    dispatcher: Arc<dyn Dispatcher>,
    store: crate::store::LearningStore,
    scorer: crate::scorer::RewardScorer,
    learning_dispatcher: Arc<crate::dispatcher::LearningDispatcher>,
}

impl FeedbackCollector {
    pub fn new(
        dispatcher: Arc<dyn Dispatcher>,
        store: crate::store::LearningStore,
        scorer: crate::scorer::RewardScorer,
        learning_dispatcher: Arc<crate::dispatcher::LearningDispatcher>,
    ) -> Self {
        Self { dispatcher, store, scorer, learning_dispatcher }
    }

    /// Subscribe to the core Dispatcher and process AgentEvents in a loop.
    /// Call this in a spawned tokio task. Returns when the stream ends or cancellation is signalled.
    pub async fn run(&self) -> Result<(), crate::error::LearningError> {
        let mut stream = self.dispatcher
            .subscribe()
            .await
            .map_err(|e| crate::error::LearningError::Dispatcher(e.to_string()))?;

        while let Some(event) = stream.next().await {
            match event {
                AgentEvent::TaskCompleted { agent_id, result } => {
                    let outcome = if result.success {
                        crate::types::Outcome::Success
                    } else {
                        crate::types::Outcome::Failure { reason: result.output.clone() }
                    };
                    let scoring_ctx = crate::scorer::ScoringContext {
                        outcome: outcome.clone(),
                        user_rating: None,
                        tool_error_count: None,
                        total_tool_calls: None,
                        retry_count: None,
                        max_retries: None,
                    };
                    let reward = self.scorer.score(&scoring_ctx);
                    let entry = crate::types::FeedbackEntry {
                        id: uuid::Uuid::new_v4(),
                        agent_id,
                        prompt_ref: PromptRef::new(),
                        skill_name: None,
                        model: String::new(),
                        outcome,
                        reward: Some(reward),
                        timestamp: chrono::Utc::now(),
                    };
                    tracing::debug!(?entry, "recording feedback for TaskCompleted");
                    if let Err(e) = self.store.record_feedback(&entry) {
                        tracing::warn!(error = %e, "failed to record feedback entry");
                    } else {
                        let _ = self.learning_dispatcher.publish(crate::types::LearningEvent::FeedbackRecorded {
                            agent_id: entry.agent_id.clone(),
                            reward: entry.reward.unwrap_or(0.0),
                        });
                    }
                }
                AgentEvent::ToolCall { agent_id, tool: _ } => {
                    // TODO: AgentEvent::ToolCall carries no result field. Record as Success until a
                    // ToolCallResult variant is added to the event schema. At that point, check the
                    // result and emit Outcome::ToolError when the tool reports failure.
                    let outcome = crate::types::Outcome::Success;
                    let scoring_ctx = crate::scorer::ScoringContext {
                        outcome: outcome.clone(),
                        user_rating: None,
                        tool_error_count: Some(0),
                        total_tool_calls: Some(1),
                        retry_count: None,
                        max_retries: None,
                    };
                    let reward = self.scorer.score(&scoring_ctx);
                    let entry = crate::types::FeedbackEntry {
                        id: uuid::Uuid::new_v4(),
                        agent_id,
                        prompt_ref: PromptRef::new(),
                        skill_name: None,
                        model: String::new(),
                        outcome,
                        reward: Some(reward),
                        timestamp: chrono::Utc::now(),
                    };
                    tracing::debug!(?entry, "recording feedback for ToolCall");
                    if let Err(e) = self.store.record_feedback(&entry) {
                        tracing::warn!(error = %e, "failed to record feedback entry");
                    } else {
                        let _ = self.learning_dispatcher.publish(crate::types::LearningEvent::FeedbackRecorded {
                            agent_id: entry.agent_id.clone(),
                            reward: entry.reward.unwrap_or(0.0),
                        });
                    }
                }
                // All other events (e.g., HookFired, ModelRequest, MemoryWrite, TaskAssigned) are silently ignored.
                _ => {}
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use async_trait::async_trait;
    use agent007_core::dispatcher::{Dispatcher, EventStream};
    use agent007_core::error::CoreError;
    use agent007_core::events::{AgentEvent, ToolCall};
    use agent007_core::task::TaskResult;
    use agent007_core::types::AgentId;
    use agent007_memory::store::MemoryStore;
    use futures::stream;
    use tempfile::TempDir;
    use uuid::Uuid;

    // ── Mock Dispatcher ──────────────────────────────────────────────────────────
    // Pattern: pre-load a list of events. subscribe() returns a finite stream over
    // those events. The stream ends naturally after all events are yielded, so the
    // collector's run() loop returns without needing external cancellation.

    struct MockDispatcher {
        events: Mutex<Vec<AgentEvent>>,
    }

    impl MockDispatcher {
        fn with_events(events: Vec<AgentEvent>) -> Arc<Self> {
            Arc::new(Self { events: Mutex::new(events) })
        }
    }

    #[async_trait]
    impl Dispatcher for MockDispatcher {
        async fn publish(&self, _event: AgentEvent) -> Result<(), CoreError> {
            Ok(())
        }

        async fn subscribe(&self) -> Result<EventStream, CoreError> {
            let events = self.events.lock().unwrap().clone();
            Ok(Box::pin(stream::iter(events)))
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────────

    fn make_store_and_ms() -> (crate::store::LearningStore, Arc<MemoryStore>, TempDir) {
        let dir = TempDir::new().unwrap();
        let ms = Arc::new(MemoryStore::new(dir.path()));
        let scoped = ms.scoped("learning");
        (crate::store::LearningStore::new(scoped), ms, dir)
    }

    fn make_learning_dispatcher() -> Arc<crate::dispatcher::LearningDispatcher> {
        Arc::new(crate::dispatcher::LearningDispatcher::new(64))
    }

    async fn run_with_events(events: Vec<AgentEvent>) -> (Arc<MemoryStore>, TempDir) {
        let (store, ms, dir) = make_store_and_ms();
        let dispatcher: Arc<dyn Dispatcher> = MockDispatcher::with_events(events);
        let collector = FeedbackCollector::new(
            dispatcher,
            store,
            crate::scorer::RewardScorer::default(),
            make_learning_dispatcher(),
        );
        collector.run().await.unwrap();
        (ms, dir)
    }

    // ── Tests ────────────────────────────────────────────────────────────────────

    /// FeedbackCollector::new() accepts an Arc<dyn Dispatcher> and a LearningStore
    #[test]
    fn new_accepts_dispatcher_and_store() {
        let dispatcher: Arc<dyn Dispatcher> = MockDispatcher::with_events(vec![]);
        let dir = TempDir::new().unwrap();
        let ms = Arc::new(MemoryStore::new(dir.path()));
        let store = crate::store::LearningStore::new(ms.scoped("learning"));
        let _collector = FeedbackCollector::new(
            dispatcher,
            store,
            crate::scorer::RewardScorer::default(),
            make_learning_dispatcher(),
        );
        // If this compiles and runs, the constructor works.
    }

    /// On receiving AgentEvent::TaskCompleted with success=true, collector creates a FeedbackEntry
    /// with Outcome::Success, calls RewardScorer, and persists via LearningStore.
    #[tokio::test]
    async fn task_completed_success_records_feedback_with_success_outcome() {
        let agent_id = AgentId::new();
        let result = TaskResult::success(Uuid::new_v4(), "all good".to_string());
        let events = vec![AgentEvent::TaskCompleted { agent_id, result }];

        let (ms, _dir) = run_with_events(events).await;

        let index = ms.scoped("learning").read("feedback/index/__none__").unwrap();
        assert!(index.is_some(), "expected a feedback entry in store");

        let ids: Vec<String> = serde_json::from_str(&index.unwrap()).unwrap();
        assert_eq!(ids.len(), 1);
        let entry_json = ms
            .scoped("learning")
            .read(&format!("feedback/{}", ids[0]))
            .unwrap()
            .unwrap();
        let entry: crate::types::FeedbackEntry = serde_json::from_str(&entry_json).unwrap();
        assert!(
            matches!(entry.outcome, crate::types::Outcome::Success),
            "expected Success outcome, got {:?}",
            entry.outcome
        );
        assert!(entry.reward.is_some());
    }

    /// On receiving AgentEvent::TaskCompleted with success=false, collector creates a FeedbackEntry
    /// with Outcome::Failure.
    #[tokio::test]
    async fn task_completed_failure_records_feedback_with_failure_outcome() {
        let agent_id = AgentId::new();
        let result = TaskResult::failure(Uuid::new_v4(), "timeout".to_string());
        let events = vec![AgentEvent::TaskCompleted { agent_id, result }];

        let (ms, _dir) = run_with_events(events).await;

        let index = ms.scoped("learning").read("feedback/index/__none__").unwrap();
        assert!(index.is_some());
        let ids: Vec<String> = serde_json::from_str(&index.unwrap()).unwrap();
        assert_eq!(ids.len(), 1);
        let entry_json = ms
            .scoped("learning")
            .read(&format!("feedback/{}", ids[0]))
            .unwrap()
            .unwrap();
        let entry: crate::types::FeedbackEntry = serde_json::from_str(&entry_json).unwrap();
        assert!(matches!(entry.outcome, crate::types::Outcome::Failure { .. }));
        assert!(entry.reward.is_some());
    }

    /// On receiving AgentEvent::ToolCall, collector always records Outcome::Success
    /// regardless of args content, since AgentEvent::ToolCall carries no result field.
    #[tokio::test]
    async fn tool_call_always_records_success_outcome() {
        let agent_id = AgentId::new();
        let tool = ToolCall {
            name: "bash".to_string(),
            args: serde_json::json!({ "error": "command not found" }),
        };
        let events = vec![AgentEvent::ToolCall { agent_id, tool }];

        let (ms, _dir) = run_with_events(events).await;

        let index = ms.scoped("learning").read("feedback/index/__none__").unwrap();
        assert!(index.is_some());
        let ids: Vec<String> = serde_json::from_str(&index.unwrap()).unwrap();
        let entry_json = ms
            .scoped("learning")
            .read(&format!("feedback/{}", ids[0]))
            .unwrap()
            .unwrap();
        let entry: crate::types::FeedbackEntry = serde_json::from_str(&entry_json).unwrap();
        assert!(
            matches!(entry.outcome, crate::types::Outcome::Success),
            "expected Success outcome, got {:?}",
            entry.outcome
        );
    }

    /// On receiving AgentEvent::ToolCall without "error" in args, collector records Outcome::Success.
    #[tokio::test]
    async fn tool_call_without_error_records_success_outcome() {
        let agent_id = AgentId::new();
        let tool = ToolCall {
            name: "read_file".to_string(),
            args: serde_json::json!({ "path": "/tmp/foo.txt" }),
        };
        let events = vec![AgentEvent::ToolCall { agent_id, tool }];

        let (ms, _dir) = run_with_events(events).await;

        let index = ms.scoped("learning").read("feedback/index/__none__").unwrap();
        assert!(index.is_some());
        let ids: Vec<String> = serde_json::from_str(&index.unwrap()).unwrap();
        let entry_json = ms
            .scoped("learning")
            .read(&format!("feedback/{}", ids[0]))
            .unwrap()
            .unwrap();
        let entry: crate::types::FeedbackEntry = serde_json::from_str(&entry_json).unwrap();
        assert!(
            matches!(entry.outcome, crate::types::Outcome::Success),
            "expected Success outcome, got {:?}",
            entry.outcome
        );
    }

    /// After recording feedback, FeedbackRecorded LearningEvent is emitted via LearningDispatcher.
    #[tokio::test]
    async fn task_completed_emits_feedback_recorded_learning_event() {
        use futures::StreamExt as FuturesStreamExt;

        let agent_id = AgentId::new();
        let result = TaskResult::success(Uuid::new_v4(), "done".to_string());
        let events = vec![AgentEvent::TaskCompleted { agent_id: agent_id.clone(), result }];

        let (store, _ms, _dir) = make_store_and_ms();
        let dispatcher: Arc<dyn Dispatcher> = MockDispatcher::with_events(events);
        let learning_dispatcher = make_learning_dispatcher();
        let mut learning_stream = learning_dispatcher.subscribe();

        let collector = FeedbackCollector::new(
            dispatcher,
            store,
            crate::scorer::RewardScorer::default(),
            learning_dispatcher,
        );
        collector.run().await.unwrap();

        let received = FuturesStreamExt::next(&mut learning_stream).await;
        assert!(received.is_some(), "expected a LearningEvent to be published");
        assert!(
            matches!(received.unwrap(), crate::types::LearningEvent::FeedbackRecorded { .. }),
            "expected FeedbackRecorded event"
        );
    }

    /// Events not relevant to learning (e.g., HookFired) are silently ignored.
    #[tokio::test]
    async fn irrelevant_events_are_silently_ignored() {
        let events = vec![
            AgentEvent::HookFired {
                event: agent007_core::events::HookEvent::PostAgentRun,
            },
            AgentEvent::MemoryWrite {
                key: "ctx.md".to_string(),
                value_ref: agent007_core::types::MemoryRef::new(),
            },
        ];

        let (ms, _dir) = run_with_events(events).await;

        // No feedback entries should exist.
        let index = ms.scoped("learning").read("feedback/index/__none__").unwrap();
        assert!(index.is_none(), "expected no feedback entries for irrelevant events");
    }
}
