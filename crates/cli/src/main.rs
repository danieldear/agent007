mod config;
pub mod commands;
mod built_in_skills;
#[cfg(test)]
mod test_support;

use clap::{Parser, Subcommand};
use commands::git::GitArgs;
use commands::checkpoint::CheckpointArgs;

pub use commands::workflow::WorkflowAction;

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
    /// Initialize agent007 — create dirs, write config, register MCP, install slash commands
    Init {
        /// Re-run even if already initialized (overwrites MCP registration, re-installs commands)
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Register globally in ~/.claude/ instead of the current project's .claude/
        /// Use this if you want agent007 available in every project.
        #[arg(long, default_value_t = false)]
        global: bool,
        /// Set up Claude Code integration (.claude/settings.json, commands, agents).
        /// If neither --claude nor --cursor is specified, both are set up.
        #[arg(long, default_value_t = false)]
        claude: bool,
        /// Set up Cursor integration (.cursor/mcp.json).
        /// If no IDE flags are specified, Claude Code, Cursor, Codex, Copilot, and Zed are all set up.
        #[arg(long, default_value_t = false)]
        cursor: bool,
        /// Set up Codex integration (.codex/config.toml or ~/.codex/config.toml).
        /// If no IDE flags are specified, Claude Code, Cursor, Codex, Copilot, and Zed are all set up.
        #[arg(long, default_value_t = false)]
        codex: bool,
        /// Set up GitHub Copilot (VS Code) integration (.vscode/mcp.json).
        /// If no IDE flags are specified, Claude Code, Cursor, Codex, Copilot, and Zed are all set up.
        #[arg(long, default_value_t = false)]
        copilot: bool,
        /// Set up Zed integration (~/.config/zed/settings.json LSP entry).
        /// If no IDE flags are specified, Claude Code, Cursor, Codex, Copilot, and Zed are all set up.
        #[arg(long, default_value_t = false)]
        zed: bool,
        /// Skip all IDE integration — only create .agent007/ directory structure.
        #[arg(long, default_value_t = false, conflicts_with_all = &["claude", "cursor", "codex", "copilot", "zed"])]
        no_ide: bool,
    },
    /// Start as an MCP server (stdio transport) + web dashboard on --port (default 8007).
    /// Register with: claude mcp add agent007 /path/to/agent007 serve
    Serve {
        /// Port for the web dashboard (default: 8007).
        #[arg(long, default_value_t = 8007)]
        port: u16,
        /// Disable the web dashboard (MCP-only mode).
        #[arg(long, default_value_t = false)]
        no_dashboard: bool,
    },
    /// Manage skills
    Skill(SkillArgs),
    /// Run simulation templates
    Simulate(commands::simulate::SimulateArgs),
    /// Run the AI testing pipeline
    Test(commands::test_pipeline::TestArgs),
    /// Manage personas
    Persona(PersonaArgs),
    /// Manage git operations (branch, commit, PR, impact)
    Git(GitArgs),
    /// Manage named checkpoints (stash-based)
    Checkpoint(CheckpointArgs),
    /// Rollback to a named checkpoint
    Rollback {
        /// Checkpoint name to restore
        #[arg(long)]
        to: String,
    },
    /// Replay a past agent session (stub — Phase 3)
    Replay {
        /// Session ID to replay
        #[arg(long)]
        session: String,
        /// Model to use for replay
        #[arg(long)]
        model: String,
    },
    /// Run the iterative debug loop on failing tests
    Debug {
        /// Maximum fix iterations
        #[arg(long, default_value = "5")]
        max_iter: usize,
        /// Model to use for fix proposals
        #[arg(long, default_value = "default")]
        model: String,
    },
    /// Manage and run multi-agent workflows
    Workflow(WorkflowArgs),
    /// View the agent file-access audit log
    Audit {
        /// Show entries from the last N hours (e.g. 24h, 1h)
        #[arg(long)]
        last: Option<String>,
        /// Filter by agent name
        #[arg(long)]
        agent: Option<String>,
        /// Filter by path glob (e.g. "src/auth/**")
        #[arg(long)]
        path: Option<String>,
        /// Show only blocked (denied) entries
        #[arg(long, default_value_t = false)]
        blocked: bool,
    },
    /// Start the agent007 web dashboard server.
    #[command(name = "serve-web")]
    ServeWeb {
        /// Port to listen on (default: 8007).
        #[arg(long, default_value_t = 8007)]
        port: u16,
    },
    /// Start the web dashboard (alias for serve-web). Opens on http://localhost:<port>
    Dashboard {
        /// Port to listen on (default: 3000).
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Export or import skill/workflow bundles (.a7bundle), promote to global
    Bundle(BundleArgs),
    /// Start the agent007 Language Server Protocol server.
    #[command(name = "serve-lsp")]
    ServeLsp {
        /// Use stdio transport (default, for Zed / VSCode).
        #[arg(long, conflicts_with = "tcp")]
        stdio: bool,

        /// Use TCP transport on the given port (for JetBrains).
        #[arg(long, value_name = "PORT")]
        tcp: Option<u16>,
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
    /// Install a skill from GitHub or a URL
    Install {
        /// Source: "github:user/repo/path/to/skill.md" or "https://..."
        source: String,
    },
}

#[derive(Parser, Debug)]
pub struct PersonaArgs {
    #[command(subcommand)]
    pub action: PersonaAction,
}

#[derive(Subcommand, Debug)]
pub enum PersonaAction {
    /// List all available personas (built-in + user overrides)
    List,
    /// Show full details (system prompt) for a named persona
    Show {
        /// Exact persona name, e.g. Researcher
        name: String,
    },
}

#[derive(Parser, Debug)]
pub struct BundleArgs {
    #[command(subcommand)]
    pub action: BundleAction,
}

#[derive(Subcommand, Debug)]
pub enum BundleAction {
    /// Export skills and/or workflows as a portable .a7bundle file
    Export {
        /// Output file path (default: agent007-bundle.a7bundle)
        #[arg(long, short = 'o')]
        output: Option<String>,
        /// Skill triggers to include (default: all). E.g. --skill code-review --skill refactor
        #[arg(long, value_name = "TRIGGER")]
        skills: Vec<String>,
        /// Workflow names to include (default: all). E.g. --workflow tdd
        #[arg(long, value_name = "NAME")]
        workflows: Vec<String>,
    },
    /// Import a .a7bundle file into the current project
    Import {
        /// Path to the .a7bundle file
        file: String,
        /// Overwrite existing skills/workflows
        #[arg(long, default_value_t = false)]
        overwrite: bool,
        /// Import into global ~/.agent007/ instead of project-local
        #[arg(long, default_value_t = false)]
        global: bool,
    },
    /// Promote a project-local skill or workflow to ~/.agent007/ (global)
    Promote {
        /// Skill trigger to promote (e.g. code-review or /code-review)
        #[arg(long)]
        skill: Option<String>,
        /// Workflow name to promote (e.g. tdd)
        #[arg(long)]
        workflow: Option<String>,
        /// Target global home (always ~/.agent007/, flag reserved for future use)
        #[arg(long, default_value_t = true)]
        global: bool,
    },
}

#[derive(Parser, Debug)]
pub struct WorkflowArgs {
    #[command(subcommand)]
    pub action: WorkflowAction,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // MCP/LSP clients parse stdio strictly; keep logs off stdout and
    // default stdio server modes to quiet logging.
    if matches!(cli.command, Commands::Serve { .. } | Commands::ServeLsp { .. }) {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_max_level(tracing::Level::ERROR)
            .init();
    } else {
        tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    }
    let config = std::sync::Arc::new(crate::config::Config::load()?);
    match cli.command {
        Commands::Init { force, global, claude, cursor, codex, copilot, zed, no_ide } => {
            let (do_claude, do_cursor, do_codex, do_copilot, do_zed) = if no_ide {
                (false, false, false, false, false)
            } else if !claude && !cursor && !codex && !copilot && !zed {
                (true, true, true, true, true)
            } else {
                (claude, cursor, codex, copilot, zed)
            };
            commands::init::execute(
                config,
                force,
                global,
                do_claude,
                do_cursor,
                do_codex,
                do_copilot,
                do_zed,
            )
            .await
        }
        Commands::Run { task } => commands::run::execute(config, task).await,
        Commands::Serve { port, no_dashboard } => {
            commands::serve::execute(config, port, no_dashboard).await
        }
        Commands::Skill(s) => commands::skill::execute(config, s.action).await,
        Commands::Simulate(args) => commands::simulate::execute(config, args).await,
        Commands::Test(args) => commands::test_pipeline::execute(config, args).await,
        Commands::Persona(p) => commands::persona::execute(config, p.action).await,
        Commands::Git(g) => commands::git::execute(config, g.action).await,
        Commands::Checkpoint(c) => commands::checkpoint::execute(config, c.action).await,
        Commands::Rollback { to } => {
            let cwd = std::env::current_dir()?;
            let mut agent = agent007_git_agent::GitAgent::open(&cwd)?;
            agent.rollback_to(&to)?;
            println!("Rolled back to checkpoint '{}'", to);
            Ok(())
        }
        Commands::Replay { session, model } => {
            commands::replay::execute(config, session, model).await
        }
        Commands::Debug { max_iter, model } => {
            let stack = commands::run::build_stack(&config).await?;
            let cwd = std::env::current_dir()?;
            let git_agent = agent007_git_agent::GitAgent::open(&cwd)?;
            let debug_loop = agent007_git_agent::DebugLoop::new(max_iter, model);
            let result = debug_loop
                .run(
                    &git_agent,
                    stack.model_router.clone(),
                    stack.dispatcher.clone(),
                )
                .await?;
            if result.resolved {
                println!("All tests passing after {} iteration(s).", result.iterations);
            } else {
                println!("Debug loop exhausted ({} iterations). Diagnosis:\n{}", result.iterations, result.final_output);
            }
            Ok(())
        }
        Commands::Workflow(w) => commands::workflow::execute(config, w.action).await,
        Commands::Bundle(b) => commands::bundle::execute(config, b.action).await,
        Commands::Audit { last, agent, path, blocked } => {
            commands::audit::execute(config, last, agent, path, blocked).await
        }
        Commands::ServeWeb { port } => {
            commands::serve_web::execute(config, port).await
        }
        Commands::Dashboard { port } => {
            let url = format!("http://localhost:{port}");
            tracing::info!("Starting agent007 dashboard on {url}");
            // Attempt to open browser (best-effort, no error on failure)
            let _ = std::process::Command::new("open").arg(&url).spawn()
                .or_else(|_| std::process::Command::new("xdg-open").arg(&url).spawn())
                .or_else(|_| std::process::Command::new("start").arg(&url).spawn());
            commands::serve_web::execute(config, port).await
        }
        Commands::ServeLsp { stdio: _, tcp } => {
            let mode = if let Some(port) = tcp {
                commands::serve_lsp::TransportMode::Tcp { port }
            } else {
                commands::serve_lsp::TransportMode::Stdio
            };
            commands::serve_lsp::execute(config, mode).await
        }
        Commands::Slash(args) => {
            // Map /trigger args to skill run
            if let Some(trigger) = args.first() {
                let trigger = trigger.clone();
                let rest = args[1..].join(" ");
                // If trigger is a namespace prefix (e.g. /agent007/ or /agent007),
                // show skill list instead of erroring.
                let is_prefix = trigger == "/agent007" || trigger == "/agent007/";
                if is_prefix {
                    commands::skill::execute(config, SkillAction::List).await
                } else {
                    commands::skill::execute(config, SkillAction::Run { trigger, args: rest }).await
                }
            } else {
                commands::skill::execute(config, SkillAction::List).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use crate::commands::checkpoint::CheckpointAction;
    use crate::commands::git::GitAction;

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
    fn parse_serve_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "serve"]).unwrap();
        assert!(matches!(cli.command, Commands::Serve { port: 8007, no_dashboard: false }));
    }

    #[test]
    fn parse_serve_with_port_and_no_dashboard() {
        let cli = Cli::try_parse_from(["agent007", "serve", "--port", "9000", "--no-dashboard"]).unwrap();
        assert!(matches!(cli.command, Commands::Serve { port: 9000, no_dashboard: true }));
    }

    #[test]
    fn parse_slash_trigger_as_external() {
        // agent007 /review-pr https://github.com/org/repo/pull/42
        // clap external_subcommand captures this as Commands::Slash(vec!["/review-pr", "..."])
        let cli = Cli::try_parse_from(["agent007", "/review-pr", "https://github.com/org/repo/pull/42"]).unwrap();
        assert!(matches!(cli.command, Commands::Slash(ref args) if args[0] == "/review-pr"));
    }

    #[test]
    fn parse_persona_list_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "persona", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Persona(ref p) if matches!(p.action, PersonaAction::List)
        ));
    }

    #[test]
    fn parse_persona_show_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "persona", "show", "Researcher"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Persona(ref p) if matches!(p.action, PersonaAction::Show { ref name } if name == "Researcher")
        ));
    }

    #[test]
    fn parse_checkpoint_create_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "checkpoint", "create", "before refactor"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Checkpoint(ref c) if matches!(c.action, CheckpointAction::Create { ref name } if name == "before refactor")
        ));
    }

    #[test]
    fn parse_checkpoint_list_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "checkpoint", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Checkpoint(ref c) if matches!(c.action, CheckpointAction::List)
        ));
    }

    #[test]
    fn parse_rollback_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "rollback", "--to", "before refactor"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Rollback { ref to } if to == "before refactor"
        ));
    }

    #[test]
    fn parse_git_branch_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "git", "branch", "feature/add-mDNS"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Git(ref g) if matches!(g.action, GitAction::Branch { ref name } if name == "feature/add-mDNS")
        ));
    }

    #[test]
    fn parse_git_commit_subcommand() {
        let cli = Cli::try_parse_from([
            "agent007", "git", "commit", "implement mDNS", "--files", "src/net/mdns.rs",
        ]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Git(ref g) if matches!(g.action, GitAction::Commit { .. })
        ));
    }

    #[test]
    fn parse_git_pr_subcommand() {
        let cli = Cli::try_parse_from([
            "agent007", "git", "pr",
            "--title", "Add mDNS",
            "--body", "adds mdns",
            "--head", "feature/add-mDNS",
            "--base", "main",
        ]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Git(ref g) if matches!(g.action, GitAction::Pr { .. })
        ));
    }

    #[test]
    fn parse_git_impact_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "git", "impact", "src/auth/token.rs"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Git(ref g) if matches!(g.action, GitAction::Impact { .. })
        ));
    }

    #[test]
    fn parse_replay_subcommand() {
        let cli = Cli::try_parse_from([
            "agent007", "replay", "--session", "abc123", "--model", "ollama/llama3",
        ]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Replay { ref session, ref model } if session == "abc123" && model == "ollama/llama3"
        ));
    }

    #[test]
    fn parse_audit_subcommand_no_flags() {
        let cli = Cli::try_parse_from(["agent007", "audit"]).unwrap();
        assert!(matches!(cli.command, Commands::Audit { last: None, agent: None, path: None, blocked: false }));
    }

    #[test]
    fn parse_audit_subcommand_last_flag() {
        let cli = Cli::try_parse_from(["agent007", "audit", "--last", "24h"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Audit { ref last, .. } if last.as_deref() == Some("24h")
        ));
    }

    #[test]
    fn parse_audit_subcommand_agent_and_blocked() {
        let cli = Cli::try_parse_from(["agent007", "audit", "--agent", "WorkerAgent", "--blocked"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Audit { ref agent, blocked: true, .. } if agent.as_deref() == Some("WorkerAgent")
        ));
    }

    #[test]
    fn parse_audit_subcommand_path_filter() {
        let cli = Cli::try_parse_from(["agent007", "audit", "--path", "src/auth/**"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Audit { ref path, .. } if path.as_deref() == Some("src/auth/**")
        ));
    }

    #[test]
    fn parse_workflow_run_subcommand() {
        let cli = Cli::try_parse_from([
            "agent007", "workflow", "run", "tdd-feature", "--task", "add auth"
        ]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Workflow(ref w) if matches!(
                &w.action,
                WorkflowAction::Run { name, task }
                if name == "tdd-feature" && task == "add auth"
            )
        ));
    }

    #[test]
    fn parse_workflow_list_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "workflow", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Workflow(ref w) if matches!(w.action, WorkflowAction::List)));
    }

    #[test]
    fn parse_workflow_validate_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "workflow", "validate", "my-flow"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Workflow(ref w) if matches!(&w.action, WorkflowAction::Validate { name } if name == "my-flow")
        ));
    }

    #[test]
    fn parse_workflow_show_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "workflow", "show", "my-flow"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Workflow(ref w) if matches!(&w.action, WorkflowAction::Show { name } if name == "my-flow")
        ));
    }
}
