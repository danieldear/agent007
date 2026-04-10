// crates/personas/src/lib.rs
pub mod error;
pub mod loader;
pub mod registry;

pub use error::PersonaError;
pub use registry::PersonaRegistry;
