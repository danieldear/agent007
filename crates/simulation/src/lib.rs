pub mod error;
pub mod types;
pub mod loader;
pub mod simulator;
pub mod pipeline;

pub use error::SimulationError;
pub use types::{
    OutputConfig, ScenarioDef, ScenarioFailure, SimulationReport, SimulationTemplate,
    SystemUnderTest, ValidationConfig,
};
pub use loader::TemplateLoader;
pub use pipeline::SimulationPipeline;
