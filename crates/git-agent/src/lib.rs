// crates/git-agent/src/lib.rs
pub mod error;
pub mod agent;
pub mod impact;
pub mod pr;
pub mod debug_loop;

pub use error::GitAgentError;
pub use agent::GitAgent;
pub use debug_loop::{DebugLoop, DebugLoopResult};
