//! Integration tests: WorkflowRunner → OrchestratorAgent → ModelRouter → MockProvider
//!
//! These tests exercise the full stack path that a real agent007 run takes:
//!   workflow definition → WorkflowRunner → (per-step) OrchestratorAgent → ModelRouter → provider
//!
//! They use MockProvider to remain fast and deterministic without network calls.

use std::sync::Arc;

use agent007_core::{
    dispatcher::{Dispatcher, LocalDispatcher},
    persona::{NoOpPersonaProvider, PersonaProvider},
};
use agent007_models::{MockProvider, ModelProvider, ModelRouter};
use agent007_workflows::{WorkflowDef, WorkflowRunner};
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn make_runner(response: &str) -> (WorkflowRunner, Arc<LocalDispatcher>) {
    let dispatcher = LocalDispatcher::new(128);
    let mock = Arc::new(MockProvider::new(response, "mock"));
    let mut router = ModelRouter::new("mock");
    router.register("mock", Arc::clone(&mock) as Arc<dyn ModelProvider>);

    let runner = WorkflowRunner::new(
        Arc::new(NoOpPersonaProvider) as Arc<dyn PersonaProvider>,
        Arc::new(router),
        Arc::clone(&dispatcher) as Arc<dyn Dispatcher>,
    );
    (runner, dispatcher)
}

fn single_step_workflow(step_id: &str, agent: &str, output: &str) -> WorkflowDef {
    serde_yaml::from_str(&format!(
        r#"
name: "Test Workflow"
steps:
  - id: {step_id}
    agent: {agent}
    prompt: "Do the task: {{{{task}}}}"
    output: {output}
"#
    ))
    .unwrap()
}

fn two_step_workflow() -> WorkflowDef {
    serde_yaml::from_str(
        r#"
name: "Two-Step Workflow"
steps:
  - id: research
    agent: Researcher
    prompt: "Research: {{task}}"
    output: notes

  - id: implement
    agent: Coder
    prompt: "Implement based on: {{notes}}"
    output: code
    depends_on: [research]
    inputs: [notes]
"#,
    )
    .unwrap()
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Single-step workflow completes successfully and returns the provider output.
#[tokio::test]
async fn single_step_workflow_runs_end_to_end() {
    let (runner, _dispatcher) = make_runner("step output text");
    let def = single_step_workflow("step1", "Researcher", "result");

    let result = runner.run(&def, "build a calculator").await.unwrap();

    assert_eq!(result.steps_completed, 1);
    assert_eq!(result.steps_total, 1);
    assert!(
        result.outputs.get("result").map_or(false, |v| v.contains("step output text")),
        "expected output to contain provider response; got {:?}",
        result.outputs
    );
}

/// Two sequential steps — second step depends on first and receives its output as input.
#[tokio::test]
async fn two_step_sequential_workflow_executes_in_order() {
    let (runner, _dispatcher) = make_runner("mock response");
    let def = two_step_workflow();

    let result = runner.run(&def, "write a REST API").await.unwrap();

    assert_eq!(result.steps_completed, 2);
    assert_eq!(result.steps_total, 2);
    // Both output keys must be present
    assert!(result.outputs.contains_key("notes"), "notes output missing");
    assert!(result.outputs.contains_key("code"), "code output missing");
}

/// Workflow runner publishes events to the dispatcher for each step.
#[tokio::test]
async fn workflow_runner_emits_agent_events() {
    use futures::StreamExt;

    let (runner, dispatcher) = make_runner("event-test-response");
    let mut events = dispatcher.subscribe().await.unwrap();
    let def = single_step_workflow("s1", "Coder", "output");

    runner.run(&def, "task").await.unwrap();

    // At least one event must have been published
    let event = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        events.next(),
    )
    .await
    .expect("timed out waiting for event")
    .expect("event stream ended");

    // The event should be either a ModelRequest or TaskCompleted
    use agent007_core::events::AgentEvent;
    assert!(
        matches!(event, AgentEvent::ModelRequest { .. } | AgentEvent::TaskCompleted { .. }),
        "unexpected event variant: {:?}",
        event
    );
}

/// Validate DAG: unknown `depends_on` reference returns an error before execution.
#[tokio::test]
async fn workflow_with_unknown_dependency_fails_validation() {
    let (runner, _) = make_runner("unused");
    let def: WorkflowDef = serde_yaml::from_str(
        r#"
name: "Bad Deps"
steps:
  - id: step1
    agent: Coder
    prompt: "do {{task}}"
    output: out
    depends_on: [nonexistent_step]
"#,
    )
    .unwrap();

    let err = runner.run(&def, "test").await.unwrap_err();
    assert!(
        matches!(err, agent007_workflows::WorkflowError::UnknownInput { .. }),
        "expected UnknownInput error, got: {:?}",
        err
    );
}

/// Schema validation is enforced: a step missing both `prompt` and `skill` errors at load time.
#[tokio::test]
async fn schema_validation_rejects_step_without_prompt_or_skill() {
    use agent007_workflows::WorkflowError;

    let def: WorkflowDef = serde_yaml::from_str(
        r#"
name: "Schema Bad"
steps:
  - id: s1
    agent: Coder
    output: result
"#,
    )
    .unwrap();

    let result = def.validate_schema();
    assert!(
        matches!(result, Err(WorkflowError::SchemaError { .. })),
        "expected SchemaError, got {:?}",
        result
    );
}

/// Workflow results saved to RunStore are accessible by run ID.
#[tokio::test]
async fn workflow_result_saved_to_run_store() {
    use agent007_core::RunStore;

    let tmp = TempDir::new().unwrap();
    let run_store = Arc::new(RunStore::new(tmp.path()));
    let run = run_store.create_run("test-wf", "count to 3", "mock", None).unwrap();

    let (runner, _) = make_runner("1, 2, 3");
    let runner_with_store = runner.for_run(Arc::clone(&run_store), &run.id);
    let def = single_step_workflow("count", "Coder", "sequence");

    let result = runner_with_store.run(&def, "count to 3").await.unwrap();
    run_store.finish_run(&run.id, true, "done").unwrap();

    assert_eq!(result.steps_completed, 1);

    // Verify the run metadata was persisted
    let detail = run_store.load_run(&run.id).unwrap();
    assert_eq!(detail.metadata.task, "count to 3");
    assert_eq!(detail.metadata.status, agent007_core::RunStatus::Succeeded);
}
