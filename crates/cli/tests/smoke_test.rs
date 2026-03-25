use agent007_core::{
    dispatcher::{Dispatcher, LocalDispatcher},
    events::AgentEvent,
    orchestrator::OrchestratorAgent,
    task::Task,
    types::PromptStore,
};
use agent007_models::{MockProvider, ModelProvider, ModelRouter};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn smoke_test_agents_run_and_emit_events() {
    let d = LocalDispatcher::new(64);
    let mut events = d.subscribe().await.unwrap();

    let mock = Arc::new(MockProvider::new("42", "mock"));
    let mut router = ModelRouter::new("mock");
    router.register("mock", Arc::clone(&mock) as Arc<dyn ModelProvider>);
    let store = Arc::new(Mutex::new(PromptStore::default()));
    let token = CancellationToken::new();

    let orch = OrchestratorAgent::new(
        Arc::clone(&d) as Arc<dyn Dispatcher>,
        Arc::new(router),
        store,
        token,
        4,
    );

    let result = orch.run(Task::new("what is 6 times 7?")).await.unwrap();

    // Verify result
    assert!(result.success);
    assert_eq!(result.output, "42");

    // Verify ModelRequest event emitted with opaque prompt ref (no raw prompt text)
    use futures::StreamExt;
    let e1 = events.next().await.unwrap();
    assert!(matches!(e1, AgentEvent::ModelRequest { .. }),
        "expected ModelRequest event, got {:?}", e1);

    // Verify TaskCompleted event emitted
    let e2 = events.next().await.unwrap();
    assert!(matches!(e2, AgentEvent::TaskCompleted { .. }),
        "expected TaskCompleted event, got {:?}", e2);

    // Verify the model was actually called (not short-circuited)
    assert_eq!(mock.call_count(), 1);
}
