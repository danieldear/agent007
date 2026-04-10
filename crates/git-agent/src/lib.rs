// crates/git-agent/src/lib.rs
pub mod agent;
pub mod debug_loop;
pub mod error;
pub mod impact;
pub mod pr;

pub use agent::GitAgent;
pub use debug_loop::{DebugLoop, DebugLoopResult};
pub use error::GitAgentError;
