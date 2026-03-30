//! Agent007 home directory resolution (shared by CLI and web).

use std::path::PathBuf;

/// Walk up from CWD looking for a `.agent007/` directory (like git finds `.git/`).
/// Returns `Some(path)` if found, `None` if we hit the filesystem root.
pub fn agent007_project_home() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(".agent007");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Return the global agent007 home directory (`~/.agent007/`).
pub fn agent007_global_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".agent007")
}

/// Return the agent007 home directory.
/// Priority: `AGENT007_HOME` env > project-local `.agent007/` > `~/.agent007/`
pub fn agent007_home() -> PathBuf {
    if let Ok(p) = std::env::var("AGENT007_HOME") {
        return PathBuf::from(p);
    }
    agent007_project_home().unwrap_or_else(agent007_global_home)
}
