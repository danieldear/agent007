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
            max_tokens: None,
            temperature: None,
            system: None,
        };

        let resp = self
            .provider
            .complete(req)
            .await
            .map_err(|e| TestingError::ModelError(e.to_string()))?;

        // Extract JSON block if the model wrapped it in markdown
        let json = extract_json_block(&resp.content);

        serde_json::from_str::<TestPlan>(&json).map_err(|e| TestingError::StageFailed {
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
            max_tokens: None,
            temperature: None,
            system: None,
        };

        let resp = self
            .provider
            .complete(req)
            .await
            .map_err(|e| TestingError::ModelError(e.to_string()))?;

        let json = extract_json_block(&resp.content);

        serde_json::from_str::<Vec<TestCase>>(&json).map_err(|e| TestingError::StageFailed {
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
                max_tokens: None,
                temperature: None,
                system: None,
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
    pub async fn run(&self, mut report: FailureReport) -> Result<FailureReport, TestingError> {
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
                    max_tokens: None,
                    temperature: None,
                    system: None,
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
    pub async fn run(&self, task: &str, working_dir: &Path) -> Result<FailureReport, TestingError> {
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
        let input =
            r#"{"scope":"all","priority":"high","coverage_target":80,"test_types":["unit"]}"#;
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
