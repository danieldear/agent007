use std::time::Duration;

use tempfile::NamedTempFile;
use tokio::process::Command;

use crate::error::SimulationError;
use crate::types::{ScenarioDef, SystemUnderTest, ValidationConfig};

/// Invokes the system-under-test binary for a single scenario.
pub struct Simulator;

impl Simulator {
    pub fn new() -> Self {
        Self
    }

    /// Run one scenario and return the raw stdout output.
    ///
    /// The `{{scenario_file}}` placeholder in `sut.args` is replaced with the
    /// path to a temp file containing the scenario parameters as JSON.
    /// The `{{output_file}}` placeholder is replaced with a separate temp file
    /// path; its contents are returned as the output string.
    pub async fn run_scenario(
        &self,
        sut: &SystemUnderTest,
        scenario: &ScenarioDef,
    ) -> Result<String, SimulationError> {
        // Write scenario params to a temp file
        let scenario_tmp = NamedTempFile::new()?;
        let output_tmp = NamedTempFile::new()?;
        let scenario_path = scenario_tmp.path().to_string_lossy().to_string();
        let output_path = output_tmp.path().to_string_lossy().to_string();

        let scenario_json =
            serde_json::to_string_pretty(&scenario.params).unwrap_or_else(|_| "{}".to_string());
        tokio::fs::write(scenario_tmp.path(), scenario_json.as_bytes()).await?;

        // Expand placeholders in args
        let args: Vec<String> = sut
            .args
            .iter()
            .map(|a| {
                a.replace("{{scenario_file}}", &scenario_path)
                    .replace("{{output_file}}", &output_path)
            })
            .collect();

        // Build command
        let mut cmd = Command::new(&sut.command);
        cmd.args(&args);

        if let Some(wd) = &sut.working_dir {
            cmd.current_dir(wd);
        }

        if let Some(env) = &sut.env {
            for (k, v) in env {
                cmd.env(k, v);
            }
        }

        let timeout = Duration::from_secs(sut.timeout_secs.unwrap_or(30));

        let result = tokio::time::timeout(timeout, cmd.output()).await;

        match result {
            Err(_) => Err(SimulationError::Timeout {
                name: scenario.name.clone(),
                secs: sut.timeout_secs.unwrap_or(30),
            }),
            Ok(Err(e)) => Err(SimulationError::Io(e)),
            Ok(Ok(output)) => {
                if !output.status.success() {
                    let code = output.status.code().unwrap_or(-1);
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    return Err(SimulationError::SutFailed { code, stderr });
                }

                // Prefer {{output_file}} content; fall back to stdout
                let output_file_content: Option<String> =
                    tokio::fs::read_to_string(output_tmp.path()).await.ok();

                if let Some(content) = output_file_content {
                    if !content.trim().is_empty() {
                        drop(output_tmp);
                        return Ok(content);
                    }
                }
                drop(output_tmp);

                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            }
        }
    }

    /// Validate the raw output string against the template's ValidationConfig.
    ///
    /// Checks `assert_contains` / `assert_not_contains` (substring), `max_duration_ms`
    /// (when a `duration_ms` field is present in JSON output), and `min_quality_score`
    /// (when a `quality_score` field is present in JSON output).
    /// Returns `Ok(())` if valid or if no validation constraints are set.
    pub fn validate_output(
        &self,
        scenario_name: &str,
        output: &str,
        config: &ValidationConfig,
    ) -> Result<(), SimulationError> {
        // assert_contains — exact substring match against raw output
        for needle in &config.assert_contains {
            if !output.contains(needle.as_str()) {
                return Err(SimulationError::ValidationFailed {
                    name: scenario_name.to_string(),
                    reason: format!("expected substring not found: {needle:?}"),
                });
            }
        }

        // assert_not_contains — must be absent from raw output
        for needle in &config.assert_not_contains {
            if output.contains(needle.as_str()) {
                return Err(SimulationError::ValidationFailed {
                    name: scenario_name.to_string(),
                    reason: format!("forbidden substring found: {needle:?}"),
                });
            }
        }

        // JSON-based checks: max_duration_ms and min_quality_score
        if config.max_duration_ms.is_some() || config.min_quality_score.is_some() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(output) {
                if let Some(max_dur) = config.max_duration_ms {
                    if let Some(dur) = v["duration_ms"].as_u64() {
                        if dur > max_dur {
                            return Err(SimulationError::ValidationFailed {
                                name: scenario_name.to_string(),
                                reason: format!("duration_ms {dur} exceeds max {max_dur}"),
                            });
                        }
                    }
                }

                if let Some(min_qs) = config.min_quality_score {
                    if let Some(qs) = v["quality_score"].as_f64() {
                        if qs < min_qs {
                            return Err(SimulationError::ValidationFailed {
                                name: scenario_name.to_string(),
                                reason: format!("quality_score {qs:.3} below minimum {min_qs:.3}"),
                            });
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for Simulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ScenarioDef;
    use serde_json::json;

    fn echo_sut() -> SystemUnderTest {
        SystemUnderTest {
            command: "echo".into(),
            args: vec!["hello".into()],
            timeout_secs: Some(5),
            working_dir: None,
            env: None,
        }
    }

    #[tokio::test]
    async fn simulator_runs_echo_sut() {
        let sut = echo_sut();
        let scenario = ScenarioDef {
            name: "s1".into(),
            params: json!({"x": 1}),
        };
        let sim = Simulator::new();
        let output = sim.run_scenario(&sut, &scenario).await.unwrap();
        assert!(!output.is_empty());
    }

    #[tokio::test]
    async fn simulator_detects_nonzero_exit() {
        let sut = SystemUnderTest {
            command: "false".into(),
            args: vec![],
            timeout_secs: Some(5),
            working_dir: None,
            env: None,
        };
        let scenario = ScenarioDef {
            name: "s1".into(),
            params: json!({}),
        };
        let sim = Simulator::new();
        let err = sim.run_scenario(&sut, &scenario).await.unwrap_err();
        assert!(matches!(err, SimulationError::SutFailed { .. }));
    }

    #[test]
    fn validate_passes_no_constraints() {
        let sim = Simulator::new();
        let config = ValidationConfig::default();
        assert!(sim
            .validate_output("s1", r#"{"anything": true}"#, &config)
            .is_ok());
    }

    #[test]
    fn validate_assert_contains_passes() {
        let sim = Simulator::new();
        let config = ValidationConfig {
            assert_contains: vec!["brainstorm".into(), "debug".into()],
            ..Default::default()
        };
        assert!(sim
            .validate_output("s1", "skills: /brainstorm /debug /architect", &config)
            .is_ok());
    }

    #[test]
    fn validate_assert_contains_fails_missing() {
        let sim = Simulator::new();
        let config = ValidationConfig {
            assert_contains: vec!["/missing-skill".into()],
            ..Default::default()
        };
        let err = sim
            .validate_output("s1", "skills: /brainstorm /debug", &config)
            .unwrap_err();
        assert!(matches!(err, SimulationError::ValidationFailed { .. }));
    }

    #[test]
    fn validate_assert_not_contains_fails_on_forbidden() {
        let sim = Simulator::new();
        let config = ValidationConfig {
            assert_not_contains: vec!["ERROR".into()],
            ..Default::default()
        };
        let err = sim
            .validate_output("s1", "output ERROR: something went wrong", &config)
            .unwrap_err();
        assert!(matches!(err, SimulationError::ValidationFailed { .. }));
    }

    #[test]
    fn validate_max_duration_ms_fails_on_slow() {
        let sim = Simulator::new();
        let config = ValidationConfig {
            max_duration_ms: Some(100),
            ..Default::default()
        };
        let err = sim
            .validate_output("s1", r#"{"duration_ms": 250}"#, &config)
            .unwrap_err();
        assert!(matches!(err, SimulationError::ValidationFailed { .. }));
    }

    #[test]
    fn validate_min_quality_score_fails_on_low() {
        let sim = Simulator::new();
        let config = ValidationConfig {
            min_quality_score: Some(0.8),
            ..Default::default()
        };
        let err = sim
            .validate_output("s1", r#"{"quality_score": 0.5}"#, &config)
            .unwrap_err();
        assert!(matches!(err, SimulationError::ValidationFailed { .. }));
    }

    #[test]
    fn validate_passes_within_bounds() {
        let sim = Simulator::new();
        let config = ValidationConfig {
            max_duration_ms: Some(500),
            min_quality_score: Some(0.8),
            ..Default::default()
        };
        assert!(sim
            .validate_output(
                "s1",
                r#"{"duration_ms": 200, "quality_score": 0.95}"#,
                &config
            )
            .is_ok());
    }
}
