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
