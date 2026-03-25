// crates/personas/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum PersonaError {
    #[error("IO error reading persona dir: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse persona file {path}: {reason}")]
    ParseError { path: std::path::PathBuf, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn io_error_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let err = PersonaError::Io(io_err);
        let msg = err.to_string();
        assert!(msg.contains("IO error reading persona dir"));
    }

    #[test]
    fn parse_error_display_contains_path_and_reason() {
        let err = PersonaError::ParseError {
            path: PathBuf::from("/home/.agent007/personas/bad.toml"),
            reason: "missing field `name`".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("bad.toml"));
        assert!(msg.contains("missing field"));
    }
}
