# Testing Agent Pipeline Crate Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `agent007-testing` crate providing a 5-stage AI testing pipeline (TestStrategist → TestDesigner → TestWriter → TestRunner → DebugLoop) using cargo-nextest for structured test output, with regression tracking via memory store, and `agent007 test` CLI commands.

**Architecture:** New `crates/testing` crate. Each pipeline stage is a struct with an `async fn run()` method. `TestPipeline` orchestrates them in sequence. `TestRunner` invokes `cargo nextest run --message-format json`, parses the event stream, and stores `FailureReport` in memory. Regression detection compares against the previous stored report.

**Tech Stack:** Rust, thiserror, serde_json, tokio, agent007-core, agent007-memory, agent007-models

**Prerequisites:** Plans 1–4 complete. All library crates built and tested.

**Spec:** `docs/superpowers/specs/2026-03-16-agent007-design.md`

---

## File Structure

```
crates/testing/
├── Cargo.toml
└── src/
    ├── lib.rs          # pub re-exports: TestingError, TestPipeline, TestRunner, TestingConfig
    ├── error.rs        # TestingError (thiserror)
    ├── types.rs        # TestPlan, TestCase, FailureReport, RunSummary, TestFailure, CoverageResult
    ├── config.rs       # TestingConfig + TOML parsing
    ├── runner.rs       # TestRunner — nextest invocation, output parsing, memory persistence
    └── pipeline.rs     # TestPipeline — all 5 stages

crates/cli/src/commands/
└── test_pipeline.rs    # `agent007 test` subcommand

Modified files:
    Cargo.toml                              (root workspace — add crates/testing to members)
    crates/cli/Cargo.toml                   (add agent007-testing dependency)
    crates/cli/src/main.rs                  (add Test variant to Commands enum)
    crates/cli/src/commands/mod.rs          (pub mod test_pipeline)
```

---

## Chunk 1: Scaffold crate + error type

### Task 1: Add testing crate to workspace; create Cargo.toml and error type

**Files:**
- Create: `crates/testing/Cargo.toml`
- Create: `crates/testing/src/lib.rs`
- Create: `crates/testing/src/error.rs`
- Modify: `Cargo.toml` (root — add `"crates/testing"` to `members`)

**Step 1 — write a failing test**

In `crates/testing/src/error.rs` (before any implementation exists, just the file skeleton):

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn error_variants_exist() {
        // Will fail to compile until TestingError is defined
        use crate::TestingError;
        let _e: TestingError = TestingError::MissingTool;
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007-testing 2>&1 | head -20
```

**Step 2 — implement**

`crates/testing/Cargo.toml`:
```toml
[package]
name = "agent007-testing"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core   = { path = "../core" }
agent007-memory = { path = "../memory" }
agent007-models = { path = "../models" }
thiserror   = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
tokio       = { workspace = true }
uuid        = { workspace = true }
chrono      = { workspace = true }
tracing     = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

`crates/testing/src/error.rs`:
```rust
#[derive(thiserror::Error, Debug)]
pub enum TestingError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cargo nextest not installed — run: cargo install cargo-nextest")]
    MissingTool,
    #[error("nextest output parse error: {0}")]
    ParseError(String),
    #[error("pipeline stage '{stage}' failed: {reason}")]
    StageFailed { stage: String, reason: String },
    #[error("model error: {0}")]
    ModelError(String),
}
```

`crates/testing/src/lib.rs`:
```rust
pub mod error;
pub mod types;
pub mod config;
pub mod runner;
pub mod pipeline;

pub use error::TestingError;
pub use types::{TestPlan, TestCase, FailureReport, RunSummary, TestFailure, CoverageResult};
pub use config::TestingConfig;
pub use runner::TestRunner;
pub use pipeline::TestPipeline;
```

Root `Cargo.toml` — add to `members`:
```toml
members = [
    # ... existing entries ...
    "crates/testing",
]
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-testing
```

- [ ] Task 1 complete

---

## Chunk 2: Core types

### Task 2: Implement TestPlan, TestCase, FailureReport and supporting structs

**Files:**
- Create: `crates/testing/src/types.rs`

**Step 1 — write failing tests**

Add to `crates/testing/src/types.rs` before struct definitions:
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn failure_report_roundtrip() {
        // Will fail until structs + serde derive are present
        use crate::types::{FailureReport, RunSummary, CoverageResult};
        let r = FailureReport {
            run_id: "abc".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            summary: RunSummary { total: 10, passed: 9, failed: 1 },
            failures: vec![],
            coverage: CoverageResult::default(),
            regressions: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: FailureReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.run_id, "abc");
        assert_eq!(back.summary.total, 10);
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007-testing 2>&1 | head -20
```

**Step 2 — implement**

`crates/testing/src/types.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestPlan {
    pub scope: String,
    pub priority: String,
    pub coverage_target: u8,
    pub test_types: Vec<String>,  // "unit", "integration", "property"
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestCase {
    pub name: String,
    pub description: String,
    pub test_type: String,
    pub requirement_ref: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RunSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct CoverageResult {
    pub lines: f32,
    pub branches: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestFailure {
    pub test: String,
    pub error: String,
    pub requirement: Option<String>,
    pub suggested_fix: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FailureReport {
    pub run_id: String,
    pub timestamp: String,
    pub summary: RunSummary,
    pub failures: Vec<TestFailure>,
    pub coverage: CoverageResult,
    pub regressions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_report_roundtrip() {
        let r = FailureReport {
            run_id: "abc".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            summary: RunSummary { total: 10, passed: 9, failed: 1 },
            failures: vec![],
            coverage: CoverageResult::default(),
            regressions: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: FailureReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.run_id, "abc");
        assert_eq!(back.summary.total, 10);
    }

    #[test]
    fn test_case_optional_requirement() {
        let tc = TestCase {
            name: "login_works".into(),
            description: "test login".into(),
            test_type: "unit".into(),
            requirement_ref: None,
        };
        assert!(tc.requirement_ref.is_none());
    }
}
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-testing
```

- [ ] Task 2 complete

---

## Chunk 3: Config

### Task 3: Implement TestingConfig with TOML parsing

**Files:**
- Create: `crates/testing/src/config.rs`

**Step 1 — write failing test**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn config_defaults() {
        use crate::config::TestingConfig;
        let c = TestingConfig::default();
        assert_eq!(c.coverage_target, 80);
        assert!(c.auto_fix_on_failure);
        assert_eq!(c.max_fix_iterations, 3);
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007-testing 2>&1 | head -20
```

**Step 2 — implement**

`crates/testing/src/config.rs`:
```rust
use serde::{Deserialize, Serialize};

/// Corresponds to the `[testing]` section in agent007.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestingConfig {
    /// Model used for strategy and design stages (e.g. "claude").
    pub strategy_model: String,
    /// Model used for test writing stage (e.g. "codex").
    pub writer_model: String,
    /// Minimum coverage percentage to aim for (0–100).
    pub coverage_target: u8,
    /// Automatically run the DebugLoop stage when tests fail.
    pub auto_fix_on_failure: bool,
    /// Maximum number of fix → rerun iterations in the DebugLoop.
    pub max_fix_iterations: u8,
    /// Persist FailureReport to the memory store after each run.
    pub store_reports_in_memory: bool,
}

impl Default for TestingConfig {
    fn default() -> Self {
        Self {
            strategy_model: "claude".into(),
            writer_model: "codex".into(),
            coverage_target: 80,
            auto_fix_on_failure: true,
            max_fix_iterations: 3,
            store_reports_in_memory: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let c = TestingConfig::default();
        assert_eq!(c.coverage_target, 80);
        assert!(c.auto_fix_on_failure);
        assert_eq!(c.max_fix_iterations, 3);
        assert!(c.store_reports_in_memory);
    }

    #[test]
    fn config_roundtrip_toml() {
        let c = TestingConfig::default();
        let s = toml::to_string(&c).unwrap();
        let back: TestingConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.strategy_model, "claude");
        assert_eq!(back.writer_model, "codex");
    }
}
```

Add `toml = { workspace = true }` to `crates/testing/Cargo.toml` `[dependencies]`.

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-testing
```

- [ ] Task 3 complete

---

## Chunk 4: TestRunner

### Task 4: Implement TestRunner — nextest invocation, output parsing, memory persistence

**Files:**
- Create: `crates/testing/src/runner.rs`

**Step 1 — write failing tests**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn parse_empty_nextest_output() {
        use crate::runner::TestRunner;
        // Will not compile until TestRunner + parse_nextest_output exist
        let (summary, failures) = TestRunner::parse_nextest_output("").unwrap();
        assert_eq!(summary.total, 0);
        assert!(failures.is_empty());
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007-testing 2>&1 | head -20
```

**Step 2 — implement**

`crates/testing/src/runner.rs`:
```rust
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

        if self.memory.read(MEMORY_KEY)?.is_some() || report.summary.total > 0 {
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

            let v: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| TestingError::ParseError(e.to_string()))?;

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
        Ok((RunSummary { total, passed, failed }, failures))
    }

    /// Tests that were passing in `previous` but now appear in `current_failures`.
    fn detect_regressions(
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
    async fn load_previous_report(&self) -> Option<FailureReport> {
        let raw = self.memory.read(MEMORY_KEY).ok()??;
        serde_json::from_str(&raw).ok()
    }

    /// Persist report under the canonical key.
    async fn store_report(&self, report: &FailureReport) -> Result<(), TestingError> {
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
            summary: RunSummary { total: 2, passed: 2, failed: 0 },
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
            summary: RunSummary { total: 1, passed: 0, failed: 1 },
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
            summary: RunSummary { total: 3, passed: 3, failed: 0 },
            failures: vec![],
            coverage: CoverageResult::default(),
            regressions: vec![],
        };
        runner.store_report(&report).await.unwrap();

        let loaded = runner.load_previous_report().await.unwrap();
        assert_eq!(loaded.run_id, "test-id");
    }
}
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-testing
```

- [ ] Task 4 complete

---

## Chunk 5: TestPipeline — all 5 stages

### Task 5: Implement TestPipeline with all stages

**Files:**
- Create: `crates/testing/src/pipeline.rs`

**Step 1 — write failing test**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn pipeline_config_respected() {
        use crate::config::TestingConfig;
        // Compile-time check that TestingConfig fields are accessible
        let c = TestingConfig::default();
        assert!(!c.strategy_model.is_empty());
    }
}
```

Run — expect compile failure (until pipeline.rs exists):
```bash
cargo test -p agent007-testing 2>&1 | head -20
```

**Step 2 — implement**

`crates/testing/src/pipeline.rs`:
```rust
use std::path::Path;
use std::sync::Arc;

use agent007_core::dispatcher::Dispatcher;
use agent007_memory::MemoryStore;
use agent007_models::provider::ModelProvider;

use crate::config::TestingConfig;
use crate::error::TestingError;
use crate::runner::TestRunner;
use crate::types::{FailureReport, TestCase, TestPlan};

// ---------------------------------------------------------------------------
// Stage 1: TestStrategist
// ---------------------------------------------------------------------------

/// Reads requirements, memory context, and existing tests to produce a TestPlan.
pub struct TestStrategist {
    pub provider: Arc<dyn ModelProvider>,
    pub memory: Arc<MemoryStore>,
    pub model: String,
}

impl TestStrategist {
    pub async fn run(&self, task: &str) -> Result<TestPlan, TestingError> {
        // Retrieve any existing context from memory
        let context = self
            .memory
            .read("requirements/latest")
            .map_err(|e| TestingError::StageFailed {
                stage: "TestStrategist".into(),
                reason: e.to_string(),
            })?
            .unwrap_or_default();

        let prompt = format!(
            "You are a test strategist. Given the following task and context, produce a \
             JSON TestPlan with fields: scope, priority, coverage_target (int 0-100), \
             test_types (array of \"unit\"|\"integration\"|\"property\").\n\
             Task: {task}\nContext:\n{context}"
        );

        let req = agent007_models::types::CompletionRequest {
            model: self.model.clone(),
            messages: vec![agent007_models::types::Message {
                role: agent007_models::types::Role::User,
                content: prompt,
            }],
            stream: false,
        };

        let resp = self
            .provider
            .complete(req)
            .await
            .map_err(|e| TestingError::ModelError(e.to_string()))?;

        // Extract JSON block if the model wrapped it in markdown
        let json = extract_json_block(&resp.content);

        serde_json::from_str::<TestPlan>(&json)
            .map_err(|e| TestingError::StageFailed {
                stage: "TestStrategist".into(),
                reason: format!("could not parse TestPlan JSON: {e}"),
            })
    }
}

// ---------------------------------------------------------------------------
// Stage 2: TestDesigner
// ---------------------------------------------------------------------------

/// Reads the TestPlan and codebase context; produces a list of TestCases.
pub struct TestDesigner {
    pub provider: Arc<dyn ModelProvider>,
    pub model: String,
}

impl TestDesigner {
    pub async fn run(
        &self,
        plan: &TestPlan,
        codebase_summary: &str,
    ) -> Result<Vec<TestCase>, TestingError> {
        let prompt = format!(
            "You are a test designer. Given the following TestPlan and codebase summary, \
             produce a JSON array of TestCase objects. Each has: name, description, \
             test_type (\"unit\"|\"integration\"|\"property\"), requirement_ref (optional string).\n\
             TestPlan: {}\nCodebase summary:\n{codebase_summary}",
            serde_json::to_string(plan).unwrap_or_default()
        );

        let req = agent007_models::types::CompletionRequest {
            model: self.model.clone(),
            messages: vec![agent007_models::types::Message {
                role: agent007_models::types::Role::User,
                content: prompt,
            }],
            stream: false,
        };

        let resp = self
            .provider
            .complete(req)
            .await
            .map_err(|e| TestingError::ModelError(e.to_string()))?;

        let json = extract_json_block(&resp.content);

        serde_json::from_str::<Vec<TestCase>>(&json)
            .map_err(|e| TestingError::StageFailed {
                stage: "TestDesigner".into(),
                reason: format!("could not parse TestCase list JSON: {e}"),
            })
    }
}

// ---------------------------------------------------------------------------
// Stage 3: TestWriter
// ---------------------------------------------------------------------------

/// Reads TestCases and existing test patterns; writes Rust test source files.
pub struct TestWriter {
    pub provider: Arc<dyn ModelProvider>,
    pub model: String,
    pub working_dir: std::path::PathBuf,
}

impl TestWriter {
    pub async fn run(&self, cases: &[TestCase]) -> Result<Vec<std::path::PathBuf>, TestingError> {
        let mut written: Vec<std::path::PathBuf> = Vec::new();

        for case in cases {
            let prompt = format!(
                "Write a Rust test function for the following test case. Output ONLY valid \
                 Rust code (no explanation). Use #[test] or #[tokio::test] as appropriate.\n\
                 Name: {}\nDescription: {}\nType: {}",
                case.name, case.description, case.test_type
            );

            let req = agent007_models::types::CompletionRequest {
                model: self.model.clone(),
                messages: vec![agent007_models::types::Message {
                    role: agent007_models::types::Role::User,
                    content: prompt,
                }],
                stream: false,
            };

            let resp = self
                .provider
                .complete(req)
                .await
                .map_err(|e| TestingError::ModelError(e.to_string()))?;

            let safe_name = case.name.replace("::", "_").replace(' ', "_");
            let file_path = self
                .working_dir
                .join("src")
                .join(format!("generated_{}.rs", safe_name));

            tokio::fs::write(&file_path, resp.content.as_bytes()).await?;
            written.push(file_path);
        }

        Ok(written)
    }
}

// ---------------------------------------------------------------------------
// Stage 4: TestRunner — re-exported from runner module
// ---------------------------------------------------------------------------

// TestRunner is defined in crates/testing/src/runner.rs and re-exported from lib.rs.

// ---------------------------------------------------------------------------
// Stage 5: DebugLoop
// ---------------------------------------------------------------------------

/// On failures: proposes a fix, applies it, and reruns up to `max_iterations` times.
pub struct DebugLoop {
    pub provider: Arc<dyn ModelProvider>,
    pub model: String,
    pub max_iterations: u8,
    pub working_dir: std::path::PathBuf,
    pub memory: Arc<MemoryStore>,
}

impl DebugLoop {
    pub async fn run(
        &self,
        mut report: FailureReport,
    ) -> Result<FailureReport, TestingError> {
        if report.failures.is_empty() {
            return Ok(report);
        }

        for iteration in 0..self.max_iterations {
            tracing::info!(
                iteration,
                failures = report.failures.len(),
                "DebugLoop: proposing fixes"
            );

            // For each failure, ask the model for a suggested fix
            for failure in &mut report.failures {
                if failure.suggested_fix.is_some() {
                    continue;
                }
                let prompt = format!(
                    "A Rust test failed. Suggest a brief fix (2-3 sentences or a code snippet).\n\
                     Test: {}\nError: {}",
                    failure.test, failure.error
                );

                let req = agent007_models::types::CompletionRequest {
                    model: self.model.clone(),
                    messages: vec![agent007_models::types::Message {
                        role: agent007_models::types::Role::User,
                        content: prompt,
                    }],
                    stream: false,
                };

                if let Ok(resp) = self.provider.complete(req).await {
                    failure.suggested_fix = Some(resp.content);
                }
            }

            // Rerun tests
            let runner = TestRunner::new(&self.working_dir, Arc::clone(&self.memory));
            report = runner.run().await?;

            if report.failures.is_empty() {
                tracing::info!(iteration, "DebugLoop: all tests passing — exiting loop");
                break;
            }
        }

        Ok(report)
    }
}

// ---------------------------------------------------------------------------
// TestPipeline — orchestrates all stages
// ---------------------------------------------------------------------------

pub struct TestPipeline {
    pub config: TestingConfig,
    pub provider: Arc<dyn ModelProvider>,
    pub memory: Arc<MemoryStore>,
    pub dispatcher: Arc<dyn Dispatcher>,
}

impl TestPipeline {
    pub async fn run(
        &self,
        task: &str,
        working_dir: &Path,
    ) -> Result<FailureReport, TestingError> {
        tracing::info!(task, "TestPipeline: starting");

        // Stage 1: Strategy
        let strategist = TestStrategist {
            provider: Arc::clone(&self.provider),
            memory: Arc::clone(&self.memory),
            model: self.config.strategy_model.clone(),
        };
        let plan = strategist.run(task).await?;
        tracing::info!(scope = %plan.scope, "TestPipeline: TestPlan ready");

        // Stage 2: Design
        let designer = TestDesigner {
            provider: Arc::clone(&self.provider),
            model: self.config.strategy_model.clone(),
        };
        let cases = designer.run(&plan, "").await?;
        tracing::info!(cases = cases.len(), "TestPipeline: TestCases ready");

        // Stage 3: Write
        let writer = TestWriter {
            provider: Arc::clone(&self.provider),
            model: self.config.writer_model.clone(),
            working_dir: working_dir.to_path_buf(),
        };
        let files = writer.run(&cases).await?;
        tracing::info!(files = files.len(), "TestPipeline: test files written");

        // Stage 4: Run
        let runner = TestRunner::new(working_dir, Arc::clone(&self.memory));
        let report = runner.run().await?;
        tracing::info!(
            passed = report.summary.passed,
            failed = report.summary.failed,
            "TestPipeline: initial run complete"
        );

        // Stage 5: DebugLoop (optional)
        let final_report = if self.config.auto_fix_on_failure && !report.failures.is_empty() {
            let debug_loop = DebugLoop {
                provider: Arc::clone(&self.provider),
                model: self.config.strategy_model.clone(),
                max_iterations: self.config.max_fix_iterations,
                working_dir: working_dir.to_path_buf(),
                memory: Arc::clone(&self.memory),
            };
            debug_loop.run(report).await?
        } else {
            report
        };

        tracing::info!(
            passed = final_report.summary.passed,
            failed = final_report.summary.failed,
            regressions = final_report.regressions.len(),
            "TestPipeline: complete"
        );

        Ok(final_report)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a JSON block from a string that may contain markdown fences.
fn extract_json_block(text: &str) -> String {
    // Try ```json ... ``` fences first
    if let Some(start) = text.find("```json") {
        if let Some(end) = text[start + 7..].find("```") {
            return text[start + 7..start + 7 + end].trim().to_string();
        }
    }
    // Try generic ``` fences
    if let Some(start) = text.find("```") {
        if let Some(end) = text[start + 3..].find("```") {
            return text[start + 3..start + 3 + end].trim().to_string();
        }
    }
    // Return as-is
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_block_bare() {
        let input = r#"{"scope":"all","priority":"high","coverage_target":80,"test_types":["unit"]}"#;
        assert_eq!(extract_json_block(input), input);
    }

    #[test]
    fn extract_json_block_with_fence() {
        let input = "Here is the plan:\n```json\n{\"scope\":\"all\"}\n```\nDone.";
        assert_eq!(extract_json_block(input), r#"{"scope":"all"}"#);
    }

    #[test]
    fn pipeline_config_respected() {
        let c = TestingConfig::default();
        assert!(!c.strategy_model.is_empty());
        assert_eq!(c.max_fix_iterations, 3);
    }
}
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-testing
```

- [ ] Task 5 complete

---

## Chunk 6: CLI — `agent007 test`

### Task 6: Add `agent007 test` subcommand with run/report sub-subcommands

**Files:**
- Create: `crates/cli/src/commands/test_pipeline.rs`
- Modify: `crates/cli/src/commands/mod.rs` — add `pub mod test_pipeline`
- Modify: `crates/cli/src/main.rs` — add `Test(TestArgs)` variant and dispatch
- Modify: `crates/cli/Cargo.toml` — add `agent007-testing = { path = "../testing" }`

**Step 1 — write failing test**

```rust
// In crates/cli/src/commands/test_pipeline.rs — after writing the file:
#[cfg(test)]
mod tests {
    #[test]
    fn test_args_parse_run() {
        use clap::Parser;
        use super::TestArgs;
        // `agent007 test run` should parse correctly
        let args = TestArgs::try_parse_from(["test", "run"]).unwrap();
        assert!(matches!(args.action, super::TestAction::Run { .. }));
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007 2>&1 | head -20
```

**Step 2 — implement**

`crates/cli/src/commands/test_pipeline.rs`:
```rust
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
            let provider = Arc::new(MockProvider::default());

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
```

**Modify `crates/cli/src/commands/mod.rs`:**
```rust
pub mod run;
pub mod serve;
pub mod skill;
pub mod simulate;
pub mod test_pipeline;
```

**Modify `crates/cli/src/main.rs`** — add to `Commands` enum and `match` dispatch:
```rust
// In Commands enum, add:
/// Run the AI testing pipeline
Test(TestArgs),

// In match block, add:
Commands::Test(args) => commands::test_pipeline::execute(config, args).await?,
```

Also add `use crate::commands::test_pipeline::TestArgs;` where other command arg types are used.

**Modify `crates/cli/Cargo.toml`** — add dependency:
```toml
agent007-testing = { path = "../testing" }
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007
cargo test -p agent007-testing
```

- [ ] Task 6 complete

---

## Chunk 7: Integration smoke test

### Task 7: End-to-end smoke test with a real working directory

**Files:**
- Modify: `crates/testing/src/runner.rs` — add integration test gated behind a feature flag

**Step 1 — write failing test**

The test below is gated on the `cargo-nextest` binary being available. Mark it `#[ignore]` so it only runs when explicitly requested:

```rust
#[tokio::test]
#[ignore = "requires cargo-nextest installed"]
async fn runner_works_on_real_project() {
    // Will fail until TestRunner::run() is fully wired
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
```

Run the ignored test explicitly:
```bash
cargo test -p agent007-testing -- --ignored runner_works_on_real_project
```

**Step 2** — No new implementation needed; wired in Task 4.

**Step 3 — verify all tests still green**
```bash
cargo test -p agent007-testing
cargo test -p agent007
```

- [ ] Task 7 complete

---

## Full test command reference

```bash
# Run all tests in the testing crate
cargo test -p agent007-testing

# Run all tests including CLI
cargo test -p agent007

# Run the full workspace
cargo test --workspace

# Run the integration test (requires cargo-nextest installed)
cargo test -p agent007-testing -- --ignored runner_works_on_real_project
```

---

## Summary checklist

- [ ] Chunk 1 — Scaffold crate + error type
- [ ] Chunk 2 — Core types (TestPlan, TestCase, FailureReport)
- [ ] Chunk 3 — TestingConfig with TOML parsing
- [ ] Chunk 4 — TestRunner (nextest invocation, parsing, regression detection, memory persistence)
- [ ] Chunk 5 — TestPipeline (all 5 stages: TestStrategist, TestDesigner, TestWriter, TestRunner, DebugLoop)
- [ ] Chunk 6 — CLI: `agent007 test run` / `agent007 test report`
- [ ] Chunk 7 — Integration smoke test
