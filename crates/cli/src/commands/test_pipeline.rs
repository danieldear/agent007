use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};

use agent007_core::dispatcher::LocalDispatcher;
use agent007_memory::MemoryStore;
use agent007_models::MockProvider;
use agent007_testing::{TestPipeline, TestingConfig};

use crate::config::Config;

#[derive(Parser, Debug)]
pub struct TestArgs {
    #[command(subcommand)]
    pub action: TestAction,
}

#[derive(Subcommand, Debug)]
pub enum TestAction {
    /// Run the full AI testing pipeline in the current (or specified) directory
    Run {
        /// Path to the project to test (defaults to current directory)
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Skip the DebugLoop auto-fix stage even if tests fail
        #[arg(long)]
        no_fix: bool,
    },
    /// Show the stored FailureReport
    Report {
        /// Show only the last stored report
        #[arg(long)]
        last: bool,
        /// Show only regression test names
        #[arg(long)]
        regressions: bool,
    },
}

pub async fn execute(_config: Arc<Config>, args: TestArgs) -> Result<()> {
    match args.action {
        TestAction::Run { dir, no_fix } => {
            let working_dir = dir.unwrap_or_else(|| std::env::current_dir().unwrap());

            let memory_dir = dirs_home().join(".agent007").join("memory");
            let memory = Arc::new(MemoryStore::new(&memory_dir));
            let dispatcher = LocalDispatcher::new(64);

            let mut config = TestingConfig::default();
            if no_fix {
                config.auto_fix_on_failure = false;
            }

            // Use MockProvider as a placeholder — replace with the configured live provider
            // once agent007-core provider resolution is wired up (see plan 1).
            let provider = Arc::new(MockProvider::new("", "mock"));

            let pipeline = TestPipeline {
                config,
                provider,
                memory: Arc::clone(&memory),
                dispatcher,
            };

            let task = format!("Run tests for project at {}", working_dir.display());
            let report = pipeline.run(&task, &working_dir).await?;

            println!("Run ID  : {}", report.run_id);
            println!("Total   : {}", report.summary.total);
            println!("Passed  : {}", report.summary.passed);
            println!("Failed  : {}", report.summary.failed);

            if !report.regressions.is_empty() {
                println!("\nRegressions ({}):", report.regressions.len());
                for r in &report.regressions {
                    println!("  - {r}");
                }
            }

            if !report.failures.is_empty() {
                println!("\nFailures:");
                for f in &report.failures {
                    println!("  [FAIL] {}", f.test);
                    println!("         {}", f.error);
                    if let Some(fix) = &f.suggested_fix {
                        println!("         Suggested fix: {fix}");
                    }
                }
            }
        }

        TestAction::Report { regressions, .. } => {
            let memory_dir = dirs_home().join(".agent007").join("memory");
            let memory = Arc::new(MemoryStore::new(&memory_dir));

            match memory.read("test_run/latest")? {
                None => println!("No test report found. Run `agent007 test run` first."),
                Some(raw) => {
                    let report: agent007_testing::FailureReport = serde_json::from_str(&raw)?;
                    if regressions {
                        if report.regressions.is_empty() {
                            println!("No regressions.");
                        } else {
                            for r in &report.regressions {
                                println!("{r}");
                            }
                        }
                    } else {
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    }
                }
            }
        }
    }

    Ok(())
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_args_parse_run() {
        let args = TestArgs::try_parse_from(["test", "run"]).unwrap();
        assert!(matches!(args.action, TestAction::Run { no_fix: false, .. }));
    }

    #[test]
    fn test_args_parse_run_no_fix() {
        let args = TestArgs::try_parse_from(["test", "run", "--no-fix"]).unwrap();
        assert!(matches!(args.action, TestAction::Run { no_fix: true, .. }));
    }

    #[test]
    fn test_args_parse_report_regressions() {
        let args = TestArgs::try_parse_from(["test", "report", "--regressions"]).unwrap();
        assert!(matches!(
            args.action,
            TestAction::Report { regressions: true, .. }
        ));
    }
}
