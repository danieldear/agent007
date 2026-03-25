// crates/tui/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("terminal IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("event bus subscribe error: {0}")]
    Subscribe(String),

    #[error("crossterm error: {0}")]
    Crossterm(String),
}
