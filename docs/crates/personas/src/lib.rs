// crates/personas/src/lib.rs
pub mod error;
pub mod registry;
pub mod loader;

pub use error::PersonaError;
pub use registry::PersonaRegistry;
