pub mod error;
pub mod types;
pub mod tool_executor;
pub use tool_executor::ToolExecutor;
pub mod task;
pub mod events;
pub mod dispatcher;
pub mod agent;
pub mod worker;
pub mod orchestrator;
pub mod persona;

pub use error::CoreError;
pub use types::{AgentId, PromptRef, MemoryRef, PromptStore};
pub use task::{Task, TaskResult, TaskQueue};
pub use events::AgentEvent;
pub use dispatcher::{Dispatcher, LocalDispatcher};
pub use persona::{PersonaSpec, PersonaProvider, NoOpPersonaProvider};
