use std::sync::Arc;

use anyhow::Result;

use agent007_web::WebServer;
use crate::commands::run::{
    build_stack, runtime_mode_label, selected_runtime_model, selected_runtime_provider,
    standalone_mode_available,
};
use crate::config::Config;

pub const DEFAULT_PORT: u16 = 8007;

pub async fn execute(config: Arc<Config>, port: u16) -> Result<()> {
    tracing::info!("building agent007 stack for web server…");

    let stack = build_stack(&config).await?;

    // Spawn feedback collector in background (same as the run command).
    let collector = stack.feedback_collector.clone();
    stack.tracker.spawn(async move {
        if let Err(e) = collector.run().await {
            tracing::warn!("feedback collector error: {e}");
        }
    });

    let actual_port = find_free_port(port).await;
    persist_dashboard_port(actual_port);
    tracing::info!("agent007 web dashboard starting on http://0.0.0.0:{actual_port}");
    let standalone_mode = standalone_mode_available(&config);
    let runtime_mode = runtime_mode_label(&config).to_string();
    let provider_label = match (
        selected_runtime_provider(&config),
        selected_runtime_model(&config),
    ) {
        (Some(provider), Some(model)) if provider != model => format!("{provider} / {model}"),
        (Some(provider), _) => provider,
        _ => "hosted-mcp".to_string(),
    };

    let web = WebServer::new(
        stack.dispatcher.clone(),
        stack.learning_dispatcher.clone(),
        stack.model_router.clone(),
        Some(stack.workflow_runner.clone()),
        stack.cancel.clone(),
        standalone_mode,
        runtime_mode,
        provider_label,
    );

    web.run(actual_port)
        .await
        .map_err(|e| anyhow::anyhow!("web server error: {e}"))?;

    stack.tracker.close();
    stack.tracker.wait().await;
    Ok(())
}

async fn find_free_port(preferred: u16) -> u16 {
    for offset in 0u16..50 {
        let port = preferred.wrapping_add(offset);
        let addr = format!("0.0.0.0:{port}");
        if let Ok(listener) = tokio::net::TcpListener::bind(&addr).await {
            drop(listener);
            return port;
        }
    }
    preferred
}

fn persist_dashboard_port(port: u16) {
    let memory_dir = crate::commands::run::agent007_home().join("memory");
    let store = Arc::new(agent007_memory::store::MemoryStore::new(memory_dir));
    let scoped = store.scoped("project");
    let _ = scoped.write("dashboard_port", &port.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port_is_8007() {
        assert_eq!(DEFAULT_PORT, 8007u16);
    }

    #[tokio::test]
    async fn find_free_port_returns_requested_port_when_available() {
        let port = find_free_port(8007).await;
        assert!(port >= 8007);
    }
}
