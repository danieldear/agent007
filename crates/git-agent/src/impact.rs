// crates/git-agent/src/impact.rs
use crate::error::GitAgentError;
use std::path::{Path, PathBuf};

/// Shallow grep-based impact analysis.
///
/// Extracts the module path from the given `path` (e.g. `src/auth/token.rs`
/// becomes `auth::token`) then walks all tracked `.rs` files in the repository
/// index, reading each one and checking for:
///   - `use.*<module_path>` (import pattern)
///   - `mod <stem>` (mod declaration pattern)
///
/// Returns the list of tracked `.rs` file paths that reference the given module.
pub fn impact_analysis(
    repo: &git2::Repository,
    path: &Path,
) -> Result<Vec<PathBuf>, GitAgentError> {
    let module_path = path_to_module_path(path);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Collect all tracked .rs files from the index
    let index = repo.index()?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitAgentError::ImpactAnalysis("bare repository not supported".into()))?;

    let mut affected = Vec::new();

    for entry in index.iter() {
        let entry_path = std::str::from_utf8(&entry.path)
            .map_err(|e| GitAgentError::ImpactAnalysis(e.to_string()))?
            .to_string();

        if !entry_path.ends_with(".rs") {
            continue;
        }
        // Skip the file itself
        if Path::new(&entry_path) == path {
            continue;
        }

        let full_path = workdir.join(&entry_path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let references = content.contains(&format!("use.*{}", module_path))
            || content
                .lines()
                .any(|l| l.contains("use ") && l.contains(&module_path))
            || content.lines().any(|l| {
                let trimmed = l.trim_start();
                (trimmed.starts_with("mod ") || trimmed.starts_with("pub mod "))
                    && l.contains(&stem)
            });

        if references {
            affected.push(PathBuf::from(&entry_path));
        }
    }

    Ok(affected)
}

/// Convert a file path to a Rust module path string.
///
/// Examples:
///   `src/auth/token.rs`  → `auth::token`
///   `src/net/mdns.rs`    → `net::mdns`
///   `auth/token.rs`      → `auth::token`
fn path_to_module_path(path: &Path) -> String {
    let mut components: Vec<&str> = path
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect();

    // Drop "src" prefix if present
    if components.first().copied() == Some("src") {
        components.remove(0);
    }

    // Drop ".rs" extension from last component
    if let Some(last) = components.last_mut() {
        if let Some(stripped) = last.strip_suffix(".rs") {
            *last = stripped;
        }
    }

    components.join("::")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn make_repo_with_files(files: &[(&str, &str)]) -> (tempfile::TempDir, git2::Repository) {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "t@t.com").unwrap();

        let tree_id = {
            let mut index = repo.index().unwrap();
            for (name, content) in files {
                let path = dir.path().join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&path, content).unwrap();
                index.add_path(Path::new(name)).unwrap();
            }
            index.write().unwrap();
            index.write_tree().unwrap()
        };
        {
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }

        (dir, repo)
    }

    #[test]
    fn impact_finds_file_using_module() {
        let (_dir, repo) = make_repo_with_files(&[
            ("src/auth/token.rs", "pub fn make_token() {}"),
            (
                "src/api/handler.rs",
                "use crate::auth::token;\nfn handle() {}",
            ),
            ("src/unrelated.rs", "fn foo() {}"),
        ]);
        let affected = impact_analysis(&repo, Path::new("src/auth/token.rs")).unwrap();
        assert!(
            affected.iter().any(|p| p.ends_with("handler.rs")),
            "handler.rs imports auth::token and should be in results, got {:?}",
            affected
        );
        assert!(
            !affected.iter().any(|p| p.ends_with("unrelated.rs")),
            "unrelated.rs should not appear in results"
        );
    }

    #[test]
    fn impact_finds_mod_declaration() {
        let (_dir, repo) = make_repo_with_files(&[
            ("src/auth/token.rs", "pub fn tok() {}"),
            ("src/auth/mod.rs", "pub mod token;"),
        ]);
        let affected = impact_analysis(&repo, Path::new("src/auth/token.rs")).unwrap();
        assert!(
            affected.iter().any(|p| p.ends_with("mod.rs")),
            "mod.rs declares `mod token` and should appear in results, got {:?}",
            affected
        );
    }

    #[test]
    fn impact_returns_empty_for_unused_file() {
        let (_dir, repo) = make_repo_with_files(&[
            ("src/orphan.rs", "pub fn orphan() {}"),
            ("src/main.rs", "fn main() {}"),
        ]);
        let affected = impact_analysis(&repo, Path::new("src/orphan.rs")).unwrap();
        assert!(
            affected.is_empty(),
            "orphan.rs is not imported anywhere, expected empty, got {:?}",
            affected
        );
    }
}
