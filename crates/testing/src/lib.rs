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
    evaluate_kpi_regression, summarize_scorecards, RegressionEvaluation, RegressionThresholds,
    ScorecardSummary,
};
pub use runner::TestRunner;
pub use types::{CoverageResult, FailureReport, RunSummary, TestCase, TestFailure, TestPlan};
