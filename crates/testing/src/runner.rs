use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use chrono::Utc;
use tokio::process::Command;
use uuid::Uuid;

use agent007_memory::MemoryStore;

use crate::error::TestingError;
use crate::types::{CoverageResult, FailureReport, RunSummary, TestFailure};

const MEMORY_KEY: &str = "test_run/latest";

pub struct TestRunner {
    pub working_dir: PathBuf,
    pub memory: Arc<MemoryStore>,
}

impl TestRunner {
    pub fn new(working_dir: impl Into<PathBuf>, memory: Arc<MemoryStore>) -> Self {
        Self {
            working_dir: working_dir.into(),
            memory,
        }
    }

    /// Run cargo nextest, parse JSON output, detect regressions, persist report.
    pub async fn run(&self) -> Result<FailureReport, TestingError> {
        Self::check_nextest()?;

        let output = Command::new("cargo")
            .args(["nextest", "run", "--message-format", "json"])
            .current_dir(&self.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let (summary, failures) = Self::parse_nextest_output(&stdout)?;

        let previous = self.load_previous_report().await;
        let regressions = Self::detect_regressions(&failures, previous.as_ref());

        let report = FailureReport {
            run_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now().to_rfc3339(),
            summary,
            failures,
            coverage: CoverageResult::default(),
            regressions,
        };

        if self.memory.read(MEMORY_KEY).ok().flatten().is_some() || report.summary.total > 0 {
            self.store_report(&report).await?;
        }

        Ok(report)
    }

    /// Check that cargo-nextest is installed.
    pub fn check_nextest() -> Result<(), TestingError> {
        // `cargo nextest --version` exits 0 when installed
        let status = std::process::Command::new("cargo")
            .args(["nextest", "--version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match status {
            Ok(s) if s.success() => Ok(()),
            _ => Err(TestingError::MissingTool),
        }
    }

    /// Parse the line-delimited JSON event stream emitted by `cargo nextest run --message-format json`.
    ///
    /// Each line is a JSON object with a `"type"` field.  Relevant variants:
    /// - `"test:finished"` with `"outcome"` = `"pass"` | `"fail"`
    /// - `"test:failed"` (older nextest versions)
    pub fn parse_nextest_output(
        output: &str,
    ) -> Result<(RunSummary, Vec<TestFailure>), TestingError> {
        let mut total = 0usize;
        let mut passed = 0usize;
        let mut failures: Vec<TestFailure> = Vec::new();

        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let v: serde_json::Value =
                serde_json::from_str(line).map_err(|e| TestingError::ParseError(e.to_string()))?;

            let event_type = v["type"].as_str().unwrap_or("");

            match event_type {
                "test:finished" => {
                    total += 1;
                    let outcome = v["outcome"].as_str().unwrap_or("unknown");
                    if outcome == "pass" {
                        passed += 1;
                    } else {
                        let name = v["name"].as_str().unwrap_or("unknown").to_string();
                        let error = v["stderr"]
                            .as_str()
                            .or_else(|| v["message"].as_str())
                            .unwrap_or("no output captured")
                            .to_string();
                        failures.push(TestFailure {
                            test: name,
                            error,
                            requirement: None,
                            suggested_fix: None,
                        });
                    }
                }
                // Some nextest versions emit these discrete events
                "test:passed" => {
                    total += 1;
                    passed += 1;
                }
                "test:failed" => {
                    total += 1;
                    let name = v["name"].as_str().unwrap_or("unknown").to_string();
                    let error = v["stderr"]
                        .as_str()
                        .or_else(|| v["message"].as_str())
                        .unwrap_or("no output captured")
                        .to_string();
                    failures.push(TestFailure {
                        test: name,
                        error,
                        requirement: None,
                        suggested_fix: None,
                    });
                }
                _ => {}
            }
        }

        let failed = failures.len();
        Ok((
            RunSummary {
                total,
                passed,
                failed,
            },
            failures,
        ))
    }

    /// Tests that were passing in `previous` but now appear in `current_failures`.
    pub fn detect_regressions(
        current_failures: &[TestFailure],
        previous: Option<&FailureReport>,
    ) -> Vec<String> {
        let prev = match previous {
            Some(p) => p,
            None => return vec![],
        };
        let previously_failed: std::collections::HashSet<&str> =
            prev.failures.iter().map(|f| f.test.as_str()).collect();

        current_failures
            .iter()
            .filter(|f| !previously_failed.contains(f.test.as_str()))
            .map(|f| f.test.clone())
            .collect()
    }

    /// Load previous FailureReport from memory.
    pub async fn load_previous_report(&self) -> Option<FailureReport> {
        let raw = self.memory.read(MEMORY_KEY).ok()??;
        serde_json::from_str(&raw).ok()
    }

    /// Persist report under the canonical key.
    pub async fn store_report(&self, report: &FailureReport) -> Result<(), TestingError> {
        let json = serde_json::to_string_pretty(report)
            .map_err(|e| TestingError::ParseError(e.to_string()))?;
        self.memory
            .write(MEMORY_KEY, &json)
            .map_err(|e| TestingError::Io(std::io::Error::other(e.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_nextest_output() {
        let (summary, failures) = TestRunner::parse_nextest_output("").unwrap();
        assert_eq!(summary.total, 0);
        assert!(failures.is_empty());
    }

    #[test]
    fn parse_passing_event() {
        let line = r#"{"type":"test:finished","name":"my_crate::foo","outcome":"pass"}"#;
        let (summary, failures) = TestRunner::parse_nextest_output(line).unwrap();
        assert_eq!(summary.total, 1);
        assert_eq!(summary.passed, 1);
        assert!(failures.is_empty());
    }

    #[test]
    fn parse_failing_event() {
        let line = r#"{"type":"test:finished","name":"my_crate::bar","outcome":"fail","stderr":"panicked at 'oops'"}"#;
        let (summary, failures) = TestRunner::parse_nextest_output(line).unwrap();
        assert_eq!(summary.total, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(failures[0].test, "my_crate::bar");
        assert!(failures[0].error.contains("oops"));
    }

    #[test]
    fn detect_regressions_identifies_new_failures() {
        let previous = FailureReport {
            run_id: "prev".into(),
            timestamp: "t".into(),
            summary: RunSummary {
                total: 2,
                passed: 2,
                failed: 0,
            },
            failures: vec![],
            coverage: CoverageResult::default(),
            regressions: vec![],
        };
        let current_failures = vec![TestFailure {
            test: "foo::bar".into(),
            error: "panicked".into(),
            requirement: None,
            suggested_fix: None,
        }];
        let regressions = TestRunner::detect_regressions(&current_failures, Some(&previous));
        assert_eq!(regressions, vec!["foo::bar"]);
    }

    #[test]
    fn detect_regressions_ignores_previously_failing() {
        let previous = FailureReport {
            run_id: "prev".into(),
            timestamp: "t".into(),
            summary: RunSummary {
                total: 1,
                passed: 0,
                failed: 1,
            },
            failures: vec![TestFailure {
                test: "foo::bar".into(),
                error: "old".into(),
                requirement: None,
                suggested_fix: None,
            }],
            coverage: CoverageResult::default(),
            regressions: vec![],
        };
        let current_failures = vec![TestFailure {
            test: "foo::bar".into(),
            error: "still failing".into(),
            requirement: None,
            suggested_fix: None,
        }];
        let regressions = TestRunner::detect_regressions(&current_failures, Some(&previous));
        assert!(regressions.is_empty());
    }

    #[tokio::test]
    async fn store_and_load_report() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Arc::new(MemoryStore::new(dir.path()));
        let runner = TestRunner::new(dir.path(), Arc::clone(&memory));

        let report = FailureReport {
            run_id: "test-id".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            summary: RunSummary {
                total: 3,
                passed: 3,
                failed: 0,
            },
            failures: vec![],
            coverage: CoverageResult::default(),
            regressions: vec![],
        };
        runner.store_report(&report).await.unwrap();

        let loaded = runner.load_previous_report().await.unwrap();
        assert_eq!(loaded.run_id, "test-id");
    }

    #[tokio::test]
    #[ignore = "requires cargo-nextest installed"]
    async fn runner_works_on_real_project() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();

        let tmp = tempfile::tempdir().unwrap();
        let memory = Arc::new(MemoryStore::new(tmp.path()));
        let runner = TestRunner::new(&dir, memory);
        let report = runner.run().await.unwrap();
        assert!(report.summary.total > 0);
    }
}
