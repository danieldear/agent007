use std::sync::Arc;

use anyhow::Result;

use crate::config::Config;
use agent007_core::RunStore;

pub async fn execute(config: Arc<Config>, session: String, model: String) -> Result<()> {
    let store = RunStore::new(super::run::agent007_home().join("sessions"));
    let detail = store.load_run(&session)?;

    println!("Replaying run {}", detail.metadata.id);
    println!("  kind:     {}", detail.metadata.kind);
    println!("  started:  {}", detail.metadata.started_at);
    println!("  mode:     {}", detail.metadata.mode);
    println!(
        "  provider: {}",
        detail.metadata.provider.as_deref().unwrap_or("unknown")
    );
    println!("  replay:   {}", model);
    println!();

    if detail.metadata.task.trim().is_empty() {
        anyhow::bail!("recorded run has no task payload to replay");
    }

    let mut replay_config = (*config).clone();
    replay_config.models.default = model;

    if let Some(request) = store
        .read_json_artifact_optional::<agent007_workflows::WorkflowRunRequest>(
            &session,
            "workflow-request.json",
        )?
    {
        let workflow_ref = store
            .read_json_artifact_optional::<agent007_workflows::WorkflowSourceRef>(
                &session,
                "workflow-source.json",
            )?
            .map(|source| source.workflow_ref)
            .unwrap_or_else(|| request.workflow.clone());
        let def = agent007_workflows::WorkflowLoader::load_named_from_dirs(
            agent007_core::paths::workflow_search_dirs(),
            &workflow_ref,
        )?;
        println!("  workflow: {}", request.workflow,);
        return super::workflow::execute_workflow_run(
            Arc::new(replay_config),
            def,
            request.task,
            "workflow-replay-cli",
            None,
            Some(workflow_ref),
        )
        .await;
    }

    super::run::execute(Arc::new(replay_config), detail.metadata.task.clone()).await
}
