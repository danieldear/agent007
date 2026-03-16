use agent007_core::{
    dispatcher::{Dispatcher, LocalDispatcher},
    orchestrator::OrchestratorAgent,
    task::Task,
    types::PromptStore,
};
use agent007_models::{MockProvider, ModelProvider, ModelRouter};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let d = LocalDispatcher::new(256);
    let mock = Arc::new(MockProvider::new("Hello from agent007!", "mock"));
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

    let result = orch.run(Task::new("say hello")).await?;
    println!("Result: {}", result.output);
    Ok(())
}
