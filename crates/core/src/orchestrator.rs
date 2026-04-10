use crate::dispatcher::Dispatcher;
use crate::error::CoreError;
use crate::task::{Task, TaskResult};
use crate::types::SharedPromptStore;
use crate::worker::WorkerAgent;
use agent007_models::ModelRouter;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

pub struct OrchestratorAgent {
    dispatcher: Arc<dyn Dispatcher>,
    router: Arc<ModelRouter>,
    prompt_store: SharedPromptStore,
    cancellation: CancellationToken,
    _max_workers: usize,
}

impl OrchestratorAgent {
    pub fn new(
        dispatcher: Arc<dyn Dispatcher>,
        router: Arc<ModelRouter>,
        prompt_store: SharedPromptStore,
        cancellation: CancellationToken,
        max_workers: usize,
    ) -> Self {
        Self {
            dispatcher,
            router,
            prompt_store,
            cancellation,
            _max_workers: max_workers,
        }
    }

    #[instrument(skip(self), fields(task_id = %task.id, task_type = %task.task_type))]
    pub async fn run(&self, task: Task) -> Result<TaskResult, CoreError> {
        if self.cancellation.is_cancelled() {
            return Err(CoreError::ShuttingDown);
        }
        let provider = self.router.route(&task.task_type);
        let worker = WorkerAgent::new(
            Arc::clone(&self.dispatcher),
            provider,
            Arc::clone(&self.prompt_store),
            self.cancellation.clone(),
        );
        worker.execute(task).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::LocalDispatcher;
    use crate::events::AgentEvent;
    use crate::task::Task;
    use crate::types::PromptStore;
    use agent007_models::{MockProvider, ModelProvider, ModelRouter};
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn make_orchestrator(
        response: &str,
        token: CancellationToken,
    ) -> (OrchestratorAgent, Arc<LocalDispatcher>) {
        let d = LocalDispatcher::new(64);
        let mock = Arc::new(MockProvider::new(response, "mock"));
        let mut router = ModelRouter::new("mock");
        router.register("mock", Arc::clone(&mock) as Arc<dyn ModelProvider>);
        let store = Arc::new(Mutex::new(PromptStore::default()));
        let orch = OrchestratorAgent::new(
            Arc::clone(&d) as Arc<dyn crate::dispatcher::Dispatcher>,
            Arc::new(router),
            store,
            token,
            4,
        );
        (orch, d)
    }

    #[tokio::test]
    async fn orchestrator_returns_successful_result() {
        let token = CancellationToken::new();
        let (orch, _d) = make_orchestrator("the answer", token);
        let result = orch.run(Task::new("what is 6*7?")).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, "the answer");
    }

    #[tokio::test]
    async fn orchestrator_emits_model_request_and_task_completed_events() {
        let token = CancellationToken::new();
        let (orch, d) = make_orchestrator("done", token);
        let mut events = d.subscribe().await.unwrap();

        orch.run(Task::new("build something")).await.unwrap();

        use futures::StreamExt as FutExt;
        let e1 = FutExt::next(&mut events).await.unwrap();
        let e2 = FutExt::next(&mut events).await.unwrap();
        assert!(matches!(e1, AgentEvent::ModelRequest { .. }));
        assert!(matches!(e2, AgentEvent::TaskCompleted { .. }));
    }

    #[tokio::test]
    async fn orchestrator_respects_cancellation() {
        let token = CancellationToken::new();
        token.cancel();
        let (orch, _d) = make_orchestrator("", token);
        let result = orch.run(Task::new("any")).await;
        assert!(matches!(result, Err(crate::error::CoreError::ShuttingDown)));
    }
}
