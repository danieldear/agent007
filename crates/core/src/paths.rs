//! Agent007 home directory resolution (shared by CLI and web).

use std::path::PathBuf;

fn push_unique(dirs: &mut Vec<PathBuf>, path: PathBuf) {
    if !dirs.iter().any(|d| d == &path) {
        dirs.push(path);
    }
}

/// Return enabled pack asset directories for one agent007 home.
///
/// Invalid or unsupported pack lockfiles are ignored here so a corrupt optional
/// pack cannot prevent the core runtime from starting. Pack-management commands
/// still report the underlying lockfile error directly.
pub fn enabled_pack_asset_dirs(home: &std::path::Path, kind: &str) -> Vec<PathBuf> {
    agent007_packs::enabled_pack_roots(home)
        .into_iter()
        .map(|root| root.join(kind))
        .filter(|path| path.is_dir())
        .collect()
}

fn push_home_asset_dirs(dirs: &mut Vec<PathBuf>, home: &std::path::Path, kind: &str) {
    push_unique(dirs, home.join(kind));
    for pack_dir in enabled_pack_asset_dirs(home, kind) {
        push_unique(dirs, pack_dir);
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
        push_home_asset_dirs(&mut dirs, &PathBuf::from(home), "skills");
        return dirs;
    }
    if let Some(project) = agent007_project_home() {
        push_home_asset_dirs(&mut dirs, &project, "skills");
    }
    push_home_asset_dirs(&mut dirs, &agent007_global_home(), "skills");
    dirs
}

/// Return the ordered list of directories to search for workflows (project-local first, then global).
/// When `AGENT007_HOME` is set it acts as a complete replacement — no other dirs are searched.
pub fn workflow_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        push_home_asset_dirs(&mut dirs, &PathBuf::from(home), "workflows");
        return dirs;
    }
    if let Some(project) = agent007_project_home() {
        push_home_asset_dirs(&mut dirs, &project, "workflows");
    }
    push_home_asset_dirs(&mut dirs, &agent007_global_home(), "workflows");
    dirs
}

/// Return the ordered list of directories to search for personas (project-local first, then global).
/// When `AGENT007_HOME` is set it acts as a complete replacement — no other dirs are searched.
pub fn persona_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        push_home_asset_dirs(&mut dirs, &PathBuf::from(home), "personas");
        return dirs;
    }
    if let Some(project) = agent007_project_home() {
        push_home_asset_dirs(&mut dirs, &project, "personas");
    }
    push_home_asset_dirs(&mut dirs, &agent007_global_home(), "personas");
    dirs
}

/// Return ordered tool directories, including enabled pack overlays.
pub fn tool_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        push_home_asset_dirs(&mut dirs, &PathBuf::from(home), "tools");
        return dirs;
    }
    if let Some(project) = agent007_project_home() {
        push_home_asset_dirs(&mut dirs, &project, "tools");
    }
    push_home_asset_dirs(&mut dirs, &agent007_global_home(), "tools");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_pack_assets_are_versioned_overlays() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let skill_dir = home.join("packs/example/1.2.3/skills");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::create_dir_all(home.join("packs")).unwrap();
        std::fs::write(
            home.join("packs/lock.json"),
            r#"{
              "schema_version": 1,
              "packs": {
                "example": {
                  "id": "example",
                  "version": "1.2.3",
                  "enabled": true,
                  "installed_at": "2026-06-18T00:00:00Z",
                  "registry": "fixture",
                  "artifact_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                  "manifest_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                  "history": []
                }
              }
            }"#,
        )
        .unwrap();

        assert_eq!(enabled_pack_asset_dirs(home, "skills"), vec![skill_dir]);
        assert!(enabled_pack_asset_dirs(home, "workflows").is_empty());
    }
}
