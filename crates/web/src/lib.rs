pub mod api;
pub mod dashboard;
pub mod error;
pub mod extensions_api;
pub mod mcp_registry;
pub mod metrics;
pub mod rag_sources;
pub mod server;
pub mod tool_registry;
pub mod ws;

pub use error::WebError;
pub use server::{dashboard_bind_addr, dashboard_bind_host, WebServer};
