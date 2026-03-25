// crates/tui/src/lib.rs
pub mod error;
pub mod app;
pub mod ui;
pub mod event;

pub use error::TuiError;
pub use app::App;
pub use event::EventLoop;
