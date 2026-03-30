pub mod api;
pub mod dashboard;
pub mod error;
pub mod metrics;
pub mod server;
pub mod ws;

pub use error::WebError;
pub use server::WebServer;
