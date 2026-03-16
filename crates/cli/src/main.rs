mod config;
pub mod commands;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "agent007", version = "0.1.0", about = "Multi-agent AI orchestration")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a task with the full agent stack
    Run {
        /// The task description to execute
        task: String,
    },
    /// Manage skills
    Skill(SkillArgs),
    /// Run a simulation template (Phase 2 stub)
    Simulate {
        /// Template name
        template: String,
    },
    /// Slash-command trigger (e.g. /review-pr <args>)
    #[command(external_subcommand)]
    Slash(Vec<String>),
}

#[derive(Parser, Debug)]
pub struct SkillArgs {
    #[command(subcommand)]
    pub action: SkillAction,
}

#[derive(Subcommand, Debug)]
pub enum SkillAction {
    /// List loaded skills
    List,
    /// Add a skill file to ~/.agent007/skills/
    Add {
        /// Path to the skill markdown file
        path: String,
    },
    /// Run a skill by trigger
    Run {
        /// Skill trigger (e.g. /review-pr)
        trigger: String,
        /// Arguments passed to the skill template
        args: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = std::sync::Arc::new(crate::config::Config::load()?);
    match cli.command {
        Commands::Run { task } => commands::run::execute(config, task).await,
        Commands::Skill(s) => commands::skill::execute(config, s.action).await,
        Commands::Simulate { template } => commands::simulate::execute(config, template).await,
        Commands::Slash(args) => {
            // Map /trigger args to skill run
            if let Some(trigger) = args.first() {
                let trigger = trigger.clone();
                let rest = args[1..].join(" ");
                commands::skill::execute(config, SkillAction::Run { trigger, args: rest }).await
            } else {
                anyhow::bail!("no slash command provided")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_run_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "run", "say hello"]).unwrap();
        assert!(matches!(cli.command, Commands::Run { ref task } if task == "say hello"));
    }

    #[test]
    fn parse_skill_list_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "skill", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Skill(ref s) if matches!(s.action, SkillAction::List)));
    }

    #[test]
    fn parse_skill_add_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "skill", "add", "/path/to/skill.md"]).unwrap();
        assert!(matches!(cli.command, Commands::Skill(ref s) if matches!(s.action, SkillAction::Add { ref path } if path == "/path/to/skill.md")));
    }

    #[test]
    fn parse_slash_trigger_as_external() {
        // agent007 /review-pr https://github.com/org/repo/pull/42
        // clap external_subcommand captures this as Commands::Slash(vec!["/review-pr", "..."])
        let cli = Cli::try_parse_from(["agent007", "/review-pr", "https://github.com/org/repo/pull/42"]).unwrap();
        assert!(matches!(cli.command, Commands::Slash(ref args) if args[0] == "/review-pr"));
    }
}
