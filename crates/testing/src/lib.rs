pub mod config;
pub mod error;
pub mod pipeline;
pub mod regression;
pub mod runner;
pub mod types;

pub use config::TestingConfig;
pub use error::TestingError;
pub use pipeline::TestPipeline;
pub use regression::{
    compare_scorecard_to_baseline, compare_scorecard_windows, compare_summary_to_baseline,
    evaluate_kpi_regression, summarize_scorecards, BaselineComparison, BaselineDelta,
    BaselineThresholds, RegressionEvaluation, RegressionThresholds, ScorecardSummary,
    ScorecardWindowComparison,
};
pub use runner::TestRunner;
pub use types::{CoverageResult, FailureReport, RunSummary, TestCase, TestFailure, TestPlan};
