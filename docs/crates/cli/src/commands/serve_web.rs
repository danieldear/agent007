use std::sync::Arc;

use anyhow::Result;

use agent007_web::WebServer;
use crate::commands::run::build_stack;
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

    tracing::info!("agent007 web dashboard starting on http://0.0.0.0:{port}");

    let web = WebServer::new(
        stack.dispatcher.clone(),
        stack.learning_dispatcher.clone(),
        stack.model_router.clone(),
        stack.cancel.clone(),
    );

    web.run(port)
        .await
        .map_err(|e| anyhow::anyhow!("web server error: {e}"))?;

    stack.tracker.close();
    stack.tracker.wait().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port_is_8007() {
        assert_eq!(DEFAULT_PORT, 8007u16);
    }
}
