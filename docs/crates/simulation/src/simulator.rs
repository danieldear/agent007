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

        let scenario_json = serde_json::to_string_pretty(&scenario.params)
            .unwrap_or_else(|_| "{}".to_string());
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
    /// For JSON output the validator attempts to parse and check known fields.
    /// Returns `Ok(())` if valid or if no validation constraints are set.
    pub fn validate_output(
        &self,
        scenario_name: &str,
        output: &str,
        config: &ValidationConfig,
    ) -> Result<(), SimulationError> {
        // Try to parse as JSON for structured validation
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(output) {
            if let Some(max_err) = config.max_error_m {
                if let Some(err_m) = v["error_m"].as_f64() {
                    if err_m > max_err {
                        return Err(SimulationError::ValidationFailed {
                            name: scenario_name.to_string(),
                            reason: format!(
                                "error_m {err_m:.2} exceeds max {max_err:.2}"
                            ),
                        });
                    }
                }
            }

            if let Some(min_acc) = config.min_accuracy_percent {
                if let Some(acc) = v["accuracy_percent"].as_f64() {
                    if acc < min_acc {
                        return Err(SimulationError::ValidationFailed {
                            name: scenario_name.to_string(),
                            reason: format!(
                                "accuracy_percent {acc:.1} below minimum {min_acc:.1}"
                            ),
                        });
                    }
                }
            }

            if let Some(max_ho) = config.max_handoff_time_ms {
                if let Some(ho) = v["handoff_time_ms"].as_u64() {
                    if ho > max_ho {
                        return Err(SimulationError::ValidationFailed {
                            name: scenario_name.to_string(),
                            reason: format!(
                                "handoff_time_ms {ho} exceeds max {max_ho}"
                            ),
                        });
                    }
                }
            }

            if let Some(min_csr) = config.min_connection_success_rate {
                if let Some(csr) = v["connection_success_rate"].as_f64() {
                    if csr < min_csr {
                        return Err(SimulationError::ValidationFailed {
                            name: scenario_name.to_string(),
                            reason: format!(
                                "connection_success_rate {csr:.3} below minimum {min_csr:.3}"
                            ),
                        });
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
        let scenario = ScenarioDef { name: "s1".into(), params: json!({}) };
        let sim = Simulator::new();
        let err = sim.run_scenario(&sut, &scenario).await.unwrap_err();
        assert!(matches!(err, SimulationError::SutFailed { .. }));
    }

    #[test]
    fn validate_passes_no_constraints() {
        let sim = Simulator::new();
        let config = ValidationConfig::default();
        assert!(sim.validate_output("s1", r#"{"anything": true}"#, &config).is_ok());
    }

    #[test]
    fn validate_fails_on_high_error_m() {
        let sim = Simulator::new();
        let config = ValidationConfig { max_error_m: Some(2.0), ..Default::default() };
        let err = sim
            .validate_output("s1", r#"{"error_m": 5.5}"#, &config)
            .unwrap_err();
        assert!(matches!(err, SimulationError::ValidationFailed { .. }));
    }

    #[test]
    fn validate_passes_within_bounds() {
        let sim = Simulator::new();
        let config = ValidationConfig { max_error_m: Some(2.0), ..Default::default() };
        assert!(sim
            .validate_output("s1", r#"{"error_m": 1.5}"#, &config)
            .is_ok());
    }
}
