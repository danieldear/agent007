use anyhow::Result;
use std::sync::Arc;
use crate::config::Config;
use crate::SkillAction;

pub async fn execute(_config: Arc<Config>, _action: SkillAction) -> Result<()> {
    todo!("skill command not yet implemented")
}
