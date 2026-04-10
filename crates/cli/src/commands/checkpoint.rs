// crates/cli/src/commands/checkpoint.rs
use agent007_git_agent::GitAgent;
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct CheckpointArgs {
    #[command(subcommand)]
    pub action: CheckpointAction,
}

#[derive(Subcommand, Debug)]
pub enum CheckpointAction {
    /// Create a named checkpoint (stash) of the current working tree
    Create {
        /// Checkpoint name
        name: String,
    },
    /// List all agent007 checkpoints
    List,
}

pub async fn execute(
    _config: std::sync::Arc<crate::config::Config>,
    action: CheckpointAction,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut agent = GitAgent::open(&cwd)?;

    match action {
        CheckpointAction::Create { name } => {
            let oid = agent.checkpoint_create(&name)?;
            println!("Checkpoint '{}' created: {}", name, oid);
        }
        CheckpointAction::List => {
            let checkpoints = agent.checkpoint_list()?;
            if checkpoints.is_empty() {
                println!("No checkpoints found.");
            } else {
                for cp in checkpoints {
                    println!("  - {}", cp);
                }
            }
        }
    }
    Ok(())
}
