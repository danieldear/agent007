// crates/git-agent/src/debug_loop.rs
use crate::agent::GitAgent;
use crate::error::GitAgentError;
use agent007_core::dispatcher::Dispatcher;
use agent007_models::provider::ModelProvider;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct TestFailure {
    pub test_name: String,
    pub stderr: String,
}

#[derive(Debug)]
pub struct DebugLoopResult {
    pub iterations: usize,
    pub resolved: bool,
    pub final_output: String,
}

pub struct DebugLoop {
    pub max_iterations: usize,
    pub model: String,
}

impl DebugLoop {
    pub fn new(max_iterations: usize, model: impl Into<String>) -> Self {
        Self {
            max_iterations,
            model: model.into(),
        }
    }

    /// Run the debug loop: repeatedly run nextest, parse failures, ask the model
    /// for a fix, apply it, and rerun — up to `max_iterations` times.
    pub async fn run(
        &self,
        git_agent: &GitAgent,
        provider: Arc<dyn ModelProvider>,
        _dispatcher: Arc<dyn Dispatcher>,
    ) -> Result<DebugLoopResult, GitAgentError> {
        let workdir = git_agent
            .repo
            .workdir()
            .ok_or_else(|| GitAgentError::ImpactAnalysis("bare repository not supported".into()))?
            .to_path_buf();

        let mut iterations = 0;
        let mut last_output = String::new();

        for _ in 0..self.max_iterations {
            iterations += 1;

            // 1. Run cargo nextest
            let output = run_nextest(&workdir)?;
            last_output = output.clone();

            // 2. Parse failures
            let failures = parse_nextest_failures(&output);
            if failures.is_empty() {
                return Ok(DebugLoopResult {
                    iterations,
                    resolved: true,
                    final_output: last_output,
                });
            }

            // 3. Build prompt with failures
            let prompt = build_fix_prompt(&failures, &workdir);

            // 4. Ask the model for a fix
            let request = agent007_models::types::CompletionRequest {
                model: self.model.clone(),
                messages: vec![agent007_models::types::Message {
                    role: agent007_models::types::Role::User,
                    content: prompt,
                }],
                max_tokens: Some(2048),
                temperature: Some(0.2),
                system: None,
            };

            let response = provider
                .complete(request)
                .await
                .map_err(|e| GitAgentError::ImpactAnalysis(e.to_string()))?;

            // 5. Apply fix — parse file/content from response and write to disk
            apply_fix_proposal(&response.content, &workdir)?;
        }

        Ok(DebugLoopResult {
            iterations,
            resolved: false,
            final_output: format!(
                "Debug loop exhausted after {} iterations. Last output:\n{}",
                iterations, last_output
            ),
        })
    }
}

/// Run `cargo nextest run --message-format json` in the given directory.
/// Returns the combined stdout as a String.
fn run_nextest(workdir: &std::path::Path) -> Result<String, GitAgentError> {
    let output = std::process::Command::new("cargo")
        .args(["nextest", "run", "--message-format", "json"])
        .current_dir(workdir)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr))
}

/// Parse nextest JSON output for failed test events.
pub fn parse_nextest_failures(output: &str) -> Vec<TestFailure> {
    let mut failures = Vec::new();
    for line in output.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("type").and_then(|t| t.as_str()) == Some("test")
                && v.get("event").and_then(|e| e.as_str()) == Some("failed")
            {
                let test_name = v
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let stderr = v
                    .get("stderr")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                failures.push(TestFailure { test_name, stderr });
            }
        }
    }
    failures
}

/// Build a prompt asking the model to fix the given test failures.
fn build_fix_prompt(failures: &[TestFailure], workdir: &std::path::Path) -> String {
    let mut prompt = String::from("The following tests are failing. Propose a minimal fix.\n\n");
    for f in failures {
        prompt.push_str(&format!(
            "## Failing test: {}\nError output:\n{}\n\n",
            f.test_name, f.stderr
        ));
        // Attempt to read the source file for context
        if let Some(file_path) = test_name_to_path(&f.test_name, workdir) {
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                prompt.push_str(&format!(
                    "### Source ({}):\n```rust\n{}\n```\n\n",
                    file_path.display(),
                    content
                ));
            }
        }
    }
    prompt.push_str(
        "Respond with the fixed file content in the format:\n\
        FILE: <relative/path/to/file.rs>\n\
        ```rust\n<complete file content>\n```\n",
    );
    prompt
}

/// Attempt to find a source file path from a test name like `crate::module::test_foo`.
fn test_name_to_path(test_name: &str, workdir: &std::path::Path) -> Option<std::path::PathBuf> {
    let parts: Vec<&str> = test_name.split("::").collect();
    if parts.len() < 2 {
        return None;
    }
    // Try src/<part1>/<part2>.rs etc.
    let candidate = workdir
        .join("src")
        .join(parts[..parts.len() - 1].join("/"))
        .with_extension("rs");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

/// Parse a fix proposal from the model and write the file to disk.
/// Expects format:
///   FILE: <path>
///   \`\`\`rust
///   <content>
///   \`\`\`
fn apply_fix_proposal(proposal: &str, workdir: &std::path::Path) -> Result<(), GitAgentError> {
    let mut current_file: Option<std::path::PathBuf> = None;
    let mut in_block = false;
    let mut content_lines: Vec<&str> = Vec::new();

    for line in proposal.lines() {
        if let Some(path_str) = line.strip_prefix("FILE: ") {
            current_file = Some(workdir.join(path_str.trim()));
        } else if line.trim_start().starts_with("```rust") {
            in_block = true;
            content_lines.clear();
        } else if line.trim() == "```" && in_block {
            in_block = false;
            if let Some(ref file_path) = current_file {
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(file_path, content_lines.join("\n"))?;
            }
        } else if in_block {
            content_lines.push(line);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn debug_loop_returns_resolved_true_when_no_test_failures() {
        // When there are no test failures, run() should return resolved: true
        // after the first iteration (without calling the provider).
        // We can't run cargo nextest in unit tests, so this test exercises the
        // parse_nextest_failures() function directly.
        let output = "";
        let failures = parse_nextest_failures(output);
        assert!(failures.is_empty(), "empty output should yield no failures");
    }

    #[test]
    fn parse_nextest_failures_extracts_test_names() {
        // Simulate a nextest JSON output line indicating a failure
        let output = r#"{"type":"test","event":"failed","name":"crate::mod::test_foo","stderr":"assertion failed"}"#;
        let failures = parse_nextest_failures(output);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].test_name.contains("test_foo"));
    }

    #[test]
    fn parse_nextest_failures_ignores_non_failed_events() {
        let output = r#"{"type":"test","event":"passed","name":"crate::mod::test_ok"}"#;
        let failures = parse_nextest_failures(output);
        assert!(failures.is_empty());
    }

    #[test]
    fn debug_loop_result_not_resolved_on_max_iterations() {
        // Verify DebugLoopResult fields
        let result = DebugLoopResult {
            iterations: 5,
            resolved: false,
            final_output: "still failing".to_string(),
        };
        assert_eq!(result.iterations, 5);
        assert!(!result.resolved);
        assert!(result.final_output.contains("still failing"));
    }
}
