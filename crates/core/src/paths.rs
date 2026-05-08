//! Agent007 home directory resolution (shared by CLI and web).

use std::path::PathBuf;

fn push_unique(dirs: &mut Vec<PathBuf>, path: PathBuf) {
    if !dirs.iter().any(|d| d == &path) {
        dirs.push(path);
    }
}

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

/// Return the ordered list of directories to search for skills (project-local first, then global).
/// Matches the listing behaviour of the web dashboard and the CLI skill commands.
/// When `AGENT007_HOME` is set it acts as a complete replacement — no other dirs are searched.
pub fn skills_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        push_unique(&mut dirs, PathBuf::from(home).join("skills"));
        return dirs;
    }
    if let Some(project) = agent007_project_home() {
        push_unique(&mut dirs, project.join("skills"));
    }
    let global = agent007_global_home().join("skills");
    push_unique(&mut dirs, global);
    dirs
}

/// Return the ordered list of directories to search for workflows (project-local first, then global).
/// When `AGENT007_HOME` is set it acts as a complete replacement — no other dirs are searched.
pub fn workflow_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        push_unique(&mut dirs, PathBuf::from(home).join("workflows"));
        return dirs;
    }
    if let Some(project) = agent007_project_home() {
        push_unique(&mut dirs, project.join("workflows"));
    }
    let global = agent007_global_home().join("workflows");
    push_unique(&mut dirs, global);
    dirs
}

/// Return the ordered list of directories to search for personas (project-local first, then global).
/// When `AGENT007_HOME` is set it acts as a complete replacement — no other dirs are searched.
pub fn persona_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        push_unique(&mut dirs, PathBuf::from(home).join("personas"));
        return dirs;
    }
    if let Some(project) = agent007_project_home() {
        push_unique(&mut dirs, project.join("personas"));
    }
    let global = agent007_global_home().join("personas");
    push_unique(&mut dirs, global);
    dirs
}

/// Return the directory where new assets (skills, workflows, memory) should be written.
///
/// Preference order:
/// 1. `AGENT007_HOME` env var (explicit override)
/// 2. Project-local `.agent007/` found by walking up from CWD  ← write here if it exists
/// 3. CWD `.agent007/` — if CWD looks like a project root (has `.git/` or `Cargo.toml` etc.)
///    create it and write there, keeping project assets out of the global home
/// 4. `~/.agent007/` global fallback
pub fn agent007_write_home() -> PathBuf {
    if let Ok(p) = std::env::var("AGENT007_HOME") {
        return PathBuf::from(p);
    }
    // Already has a project-local .agent007/ — use it
    if let Some(project) = agent007_project_home() {
        return project;
    }
    // CWD looks like a project root → create .agent007/ there on first write
    if let Ok(cwd) = std::env::current_dir() {
        let is_project_root = cwd.join(".git").exists()
            || cwd.join("Cargo.toml").exists()
            || cwd.join("package.json").exists()
            || cwd.join("pyproject.toml").exists()
            || cwd.join("go.mod").exists();
        if is_project_root {
            return cwd.join(".agent007");
        }
    }
    agent007_global_home()
}
