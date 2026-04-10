use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestPlan {
    pub scope: String,
    pub priority: String,
    pub coverage_target: u8,
    pub test_types: Vec<String>, // "unit", "integration", "property"
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
            summary: RunSummary {
                total: 10,
                passed: 9,
                failed: 1,
            },
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
