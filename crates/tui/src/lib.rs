// crates/tui/src/lib.rs
pub mod app;
pub mod error;
pub mod event;
pub mod ui;

pub use app::App;
pub use error::TuiError;
pub use event::EventLoop;
