use std::sync::Arc;

use agent007_ide_bridge::server::{run_stdio, run_tcp, BridgeConfig};
use anyhow::Result;

use crate::config::Config;

#[derive(Debug, Clone)]
pub enum TransportMode {
    Stdio,
    Tcp { port: u16 },
}

pub async fn execute(config: Arc<Config>, mode: TransportMode) -> Result<()> {
    let bridge_cfg = Arc::new(BridgeConfig {
        max_agents: config.core.max_agents,
        default_model: config.models.default.clone(),
    });

    match mode {
        TransportMode::Stdio => {
            tracing::info!("agent007 LSP server starting (stdio transport)");
            run_stdio(bridge_cfg).await;
        }
        TransportMode::Tcp { port } => {
            tracing::info!("agent007 LSP server starting (TCP port {})", port);
            run_tcp(bridge_cfg, port)
                .await
                .map_err(|e| anyhow::anyhow!("LSP TCP server error: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_mode_parses_stdio() {
        let mode = TransportMode::Stdio;
        assert!(matches!(mode, TransportMode::Stdio));
    }

    #[test]
    fn transport_mode_parses_tcp() {
        let mode = TransportMode::Tcp { port: 7007 };
        assert!(matches!(mode, TransportMode::Tcp { port: 7007 }));
    }
}
