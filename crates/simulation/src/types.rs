use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Template definition — parsed from TOML files
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, Clone)]
pub struct SimulationTemplate {
    pub name: String,
    pub description: Option<String>,
    pub research_topics: Vec<String>,
    pub system_under_test: SystemUnderTest,
    pub scenarios: Vec<ScenarioDef>,
    #[serde(default)]
    pub validation: ValidationConfig,
    #[serde(default)]
    pub output: OutputConfig,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SystemUnderTest {
    /// Binary/command to invoke (e.g. "cargo", "./my_sut").
    pub command: String,
    /// Arguments; may contain `{{scenario_file}}` and `{{output_file}}` placeholders.
    #[serde(default)]
    pub args: Vec<String>,
    /// Maximum wall-clock seconds before the process is killed.
    pub timeout_secs: Option<u64>,
    /// Working directory for the subprocess (defaults to cwd).
    pub working_dir: Option<String>,
    /// Extra environment variables.
    pub env: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScenarioDef {
    pub name: String,
    /// All other fields are domain-specific parameters.
    #[serde(flatten)]
    pub params: serde_json::Value,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct ValidationConfig {
    /// Strings that must appear in the output (exact substring match).
    #[serde(default)]
    pub assert_contains: Vec<String>,
    /// Strings that must NOT appear in the output.
    #[serde(default)]
    pub assert_not_contains: Vec<String>,
    /// Maximum allowed wall-clock duration in milliseconds.
    pub max_duration_ms: Option<u64>,
    /// Minimum quality score (0.0–1.0); checked against a `quality_score` field in JSON output.
    pub min_quality_score: Option<f64>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct OutputConfig {
    /// Output format: "csv" or "json".
    pub format: Option<String>,
    /// Persist the SimulationReport to the memory store.
    pub store_in_memory: Option<bool>,
    /// Compare this run's results against the previous stored report.
    pub compare_with_previous: Option<bool>,
}

// ---------------------------------------------------------------------------
// Run results
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SimulationReport {
    pub run_id: String,
    pub template_name: String,
    pub timestamp: String,
    pub scenarios_run: usize,
    pub scenarios_passed: usize,
    pub scenarios_failed: usize,
    pub failures: Vec<ScenarioFailure>,
    pub regressions: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ScenarioFailure {
    pub scenario: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_roundtrip() {
        let r = SimulationReport {
            run_id: "r1".into(),
            template_name: "checkout-flow".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            scenarios_run: 5,
            scenarios_passed: 5,
            scenarios_failed: 0,
            failures: vec![],
            regressions: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: SimulationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.run_id, "r1");
        assert_eq!(back.template_name, "checkout-flow");
    }

    #[test]
    fn validation_config_defaults() {
        let v = ValidationConfig::default();
        assert!(v.assert_contains.is_empty());
        assert!(v.assert_not_contains.is_empty());
        assert!(v.max_duration_ms.is_none());
        assert!(v.min_quality_score.is_none());
    }

    #[test]
    fn template_parses_from_toml() {
        let toml_str = r#"
name = "smoke-test"
description = "minimal test"
research_topics = []

[system_under_test]
command = "echo"
args = ["hello"]

[[scenarios]]
name = "s1"
foo = "bar"
        "#;
        let t: SimulationTemplate = toml::from_str(toml_str).unwrap();
        assert_eq!(t.name, "smoke-test");
        assert_eq!(t.scenarios.len(), 1);
        assert_eq!(t.scenarios[0].name, "s1");
    }

    #[test]
    fn builtin_templates_parse() {
        let skills: SimulationTemplate =
            toml::from_str(include_str!("../templates/skills-smoke.toml")).unwrap();
        assert_eq!(skills.name, "skills-smoke");
        let workflows: SimulationTemplate =
            toml::from_str(include_str!("../templates/workflow-smoke.toml")).unwrap();
        assert_eq!(workflows.name, "workflow-smoke");
    }
}
