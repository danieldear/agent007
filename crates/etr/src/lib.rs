//! agent007 Embedded Tool Runtime (ETR)
//!
//! Deterministic tool execution layer for agent007 workflows.
//! L1: Native Rust built-ins (grep, json_extract, csv_slice, glob, file_stat, math, diff).
//! L2: Language-agnostic stdin/stdout JSON plugins (future).
//! L3: Gated shell execution (future, opt-in).

pub mod audit;
pub mod compactor;
pub mod dispatcher;
pub mod l1;
pub mod policy;
pub mod types;

pub use dispatcher::EtrDispatcher;
pub use types::{EtrCallRequest, EtrCallResult, EtrStatus, ToolLayer, ToolManifest};
