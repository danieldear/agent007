use anyhow::Result;
use std::sync::Arc;
use crate::config::Config;

/// Phase 2 stub — prints a "not yet implemented" message.
pub async fn execute(_config: Arc<Config>, template: String) -> Result<()> {
    println!("simulate: '{}' — not yet implemented (Phase 2)", template);
    Ok(())
}
