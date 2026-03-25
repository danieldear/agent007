// crates/cli/src/commands/git.rs
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use agent007_git_agent::GitAgent;

#[derive(Parser, Debug)]
pub struct GitArgs {
    #[command(subcommand)]
    pub action: GitAction,
}

#[derive(Subcommand, Debug)]
pub enum GitAction {
    /// Create and checkout a new branch
    Branch {
        /// Branch name
        name: String,
    },
    /// Stage files and create a commit
    Commit {
        /// Commit message
        message: String,
        /// Files to stage (space-separated)
        #[arg(long, num_args = 1.., value_delimiter = ' ')]
        files: Vec<PathBuf>,
    },
    /// Create a pull request on GitHub or GitLab
    Pr {
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
        #[arg(long)]
        head: String,
        #[arg(long)]
        base: String,
    },
    /// Show files impacted by changes to the given path
    Impact {
        /// Path to analyze (e.g. src/auth/token.rs)
        path: PathBuf,
    },
}

pub async fn execute(_config: std::sync::Arc<crate::config::Config>, action: GitAction) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let agent = GitAgent::open(&cwd)?;

    match action {
        GitAction::Branch { name } => {
            agent.create_branch(&name)?;
            println!("Switched to new branch '{}'", name);
        }
        GitAction::Commit { message, files } => {
            let file_refs: Vec<&Path> = files.iter().map(|p| p.as_path()).collect();
            let oid = agent.auto_commit(&message, &file_refs)?;
            println!("Committed: {}", oid);
        }
        GitAction::Pr { title, body, head, base } => {
            let url = agent.create_pr(&title, &body, &head, &base).await?;
            println!("PR created: {}", url);
        }
        GitAction::Impact { path } => {
            let affected = agent.impact_analysis(&path)?;
            if affected.is_empty() {
                println!("No files reference '{}'", path.display());
            } else {
                println!("Files impacted by '{}':", path.display());
                for f in affected {
                    println!("  {}", f.display());
                }
            }
        }
    }
    Ok(())
}
