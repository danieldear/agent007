use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::instrument;
use agent007_models::{CompletionRequest, Message, ModelProvider, Role};
use crate::dispatcher::Dispatcher;
use crate::error::CoreError;
use crate::events::AgentEvent;
use crate::task::{Task, TaskResult};
use crate::types::{AgentId, SharedPromptStore};

pub struct WorkerAgent {
    pub id: AgentId,
    dispatcher: Arc<dyn Dispatcher>,
    provider: Arc<dyn ModelProvider>,
    prompt_store: SharedPromptStore,
    cancellation: CancellationToken,
}

impl WorkerAgent {
    pub fn new(
        dispatcher: Arc<dyn Dispatcher>,
        provider: Arc<dyn ModelProvider>,
        prompt_store: SharedPromptStore,
        cancellation: CancellationToken,
    ) -> Self {
        Self { id: AgentId::new(), dispatcher, provider, prompt_store, cancellation }
    }

    #[instrument(skip(self), fields(worker_id = %self.id, task_id = %task.id))]
    pub async fn execute(&self, task: Task) -> Result<TaskResult, CoreError> {
        if self.cancellation.is_cancelled() {
            return Err(CoreError::ShuttingDown);
        }

        // Store prompt in shared PromptStore, emit opaque ref on event bus
        let prompt_ref = self.prompt_store.lock().unwrap().insert(task.description.clone());

        self.dispatcher.publish(AgentEvent::ModelRequest {
            provider: self.provider.name().to_string(),
            prompt_ref: prompt_ref.clone(),
            token_estimate: task.description.split_whitespace().count().saturating_mul(2),
        }).await?;

        let request = CompletionRequest {
            model: self.provider.name().to_string(),
            messages: vec![Message { role: Role::User, content: task.description.clone() }],
            max_tokens: Some(4096),
            temperature: Some(0.7),
            system: None,
        };

        let response = self.provider.complete(request).await?;

        let result = TaskResult::success(task.id, response.content);
        self.dispatcher.publish(AgentEvent::TaskCompleted {
            agent_id: self.id.clone(),
            result: result.clone(),
            skill_name: None,
            model: Some(self.provider.name().to_string()),
        }).await?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::LocalDispatcher;
    use crate::events::AgentEvent;
    use crate::task::Task;
    use crate::types::PromptStore;
    use agent007_models::MockProvider;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn worker_executes_task_via_provider() {
        let d = LocalDispatcher::new(32);
        let mut events = d.subscribe().await.unwrap();
        let provider = Arc::new(MockProvider::new("task done", "mock"));
        let store = Arc::new(Mutex::new(PromptStore::default()));
        let token = CancellationToken::new();

        let worker = WorkerAgent::new(
            Arc::clone(&d) as Arc<dyn crate::dispatcher::Dispatcher>,
            Arc::clone(&provider) as Arc<dyn agent007_models::ModelProvider>,
            store,
            token,
        );

        let task = Task::new("do something");
        let task_id = task.id;
        let result = worker.execute(task).await.unwrap();

        assert!(result.success);
        assert_eq!(result.task_id, task_id);
        assert_eq!(result.output, "task done");
        assert_eq!(provider.call_count(), 1);

        // Verify ModelRequest event emitted with opaque ref (no raw prompt)
        use futures::StreamExt as FutExt;
        let event = FutExt::next(&mut events).await.unwrap();
        assert!(matches!(event, AgentEvent::ModelRequest { .. }));
    }

    #[tokio::test]
    async fn worker_prompt_ref_is_resolvable_in_store() {
        let d = LocalDispatcher::new(32);
        let mut events = d.subscribe().await.unwrap();
        let provider = Arc::new(MockProvider::new("result", "mock"));
        let store = Arc::new(Mutex::new(PromptStore::default()));
        let store_ref = Arc::clone(&store);
        let token = CancellationToken::new();

        let worker = WorkerAgent::new(
            Arc::clone(&d) as Arc<dyn crate::dispatcher::Dispatcher>,
            provider,
            store,
            token,
        );

        worker.execute(Task::new("my prompt text")).await.unwrap();

        use futures::StreamExt as FutExt;
        let event = FutExt::next(&mut events).await.unwrap();
        if let AgentEvent::ModelRequest { prompt_ref, .. } = event {
            let locked = store_ref.lock().unwrap();
            assert_eq!(locked.get(&prompt_ref), Some("my prompt text"));
        } else {
            panic!("expected ModelRequest event");
        }
    }

    #[tokio::test]
    async fn worker_returns_shutdown_error_when_cancelled() {
        let d = LocalDispatcher::new(32);
        let provider = Arc::new(MockProvider::new("", "mock"));
        let store = Arc::new(Mutex::new(PromptStore::default()));
        let token = CancellationToken::new();
        token.cancel();
        let worker = WorkerAgent::new(
            Arc::clone(&d) as Arc<dyn crate::dispatcher::Dispatcher>,
            provider,
            store,
            token,
        );
        let result = worker.execute(Task::new("any")).await;
        assert!(matches!(result, Err(crate::error::CoreError::ShuttingDown)));
    }
}
