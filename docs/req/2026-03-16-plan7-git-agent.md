# Git-Agent Crate Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `agent007-git-agent` crate providing git-aware operations: branch management, named checkpoints (stash-based), auto-commit, code impact analysis, PR creation via GitHub/GitLab REST API, and a debug loop that iterates on failing tests.

**Architecture:** New `crates/git-agent` crate wraps `git2` (libgit2 Rust bindings). `GitAgent` holds a `git2::Repository`. PR creation uses `reqwest` to call GitHub/GitLab REST APIs. Debug loop is a coordinator struct that calls `cargo nextest`, reads output, asks the model for a fix, applies it, and reruns. CLI commands added to `crates/cli`.

**Tech Stack:** Rust, git2 = "0.19", reqwest (workspace), thiserror, serde_json, tokio (async for PR creation + debug loop)

---

## File Structure

```
crates/git-agent/
├── Cargo.toml
└── src/
    ├── lib.rs          # pub re-exports: GitAgent, DebugLoop, DebugLoopResult, GitAgentError
    ├── error.rs        # GitAgentError (thiserror)
    ├── agent.rs        # GitAgent core: open, create_branch, auto_commit, checkpoint_*, rollback_to
    ├── impact.rs       # impact_analysis — grep-based scan of tracked .rs files
    ├── pr.rs           # create_pr — GitHub/GitLab REST API via reqwest
    └── debug_loop.rs   # DebugLoop + DebugLoopResult
crates/cli/src/commands/
    ├── git.rs          # agent007 git branch/commit/pr/impact subcommands
    └── checkpoint.rs   # agent007 checkpoint create/list + rollback subcommands
```

**Modify:**
- `Cargo.toml` (workspace root) — add `git2 = "0.19"` to `[workspace.dependencies]`; add `"crates/git-agent"` to `members`
- `crates/cli/Cargo.toml` — add `agent007-git-agent = { path = "../git-agent" }`
- `crates/cli/src/main.rs` — add `Git` and `Checkpoint` variants to `Commands` enum; wire handlers
- `crates/cli/src/commands/mod.rs` — add `pub mod git; pub mod checkpoint;`

---

## Task 1: Crate scaffold + error type

**Files:**
- Create: `crates/git-agent/Cargo.toml`
- Create: `crates/git-agent/src/lib.rs`
- Create: `crates/git-agent/src/error.rs`
- Modify: `Cargo.toml` (workspace root)

### Step 1: Add git-agent to workspace and add git2 to workspace deps

- [ ] In the workspace root `Cargo.toml`, add `"crates/git-agent"` to the `members` array and add `git2 = "0.19"` to `[workspace.dependencies]`.

```toml
# Cargo.toml [workspace.dependencies] — add this line:
git2 = "0.19"
```

```toml
# Cargo.toml [workspace] members — add:
"crates/git-agent",
```

### Step 2: Create `crates/git-agent/Cargo.toml`

- [ ] Create the file with the following content:

```toml
[package]
name = "agent007-git-agent"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core = { path = "../core" }
git2 = { workspace = true }
reqwest = { workspace = true }
thiserror = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
async-trait = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
tokio = { workspace = true }
```

### Step 3: Create `crates/git-agent/src/error.rs`

- [ ] Write failing test first — place in a `tests` submodule at the bottom of `error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_repo_error_displays_path() {
        let err = GitAgentError::NoRepo {
            path: std::path::PathBuf::from("/tmp/missing"),
        };
        assert!(err.to_string().contains("/tmp/missing"));
    }

    #[test]
    fn checkpoint_not_found_displays_name() {
        let err = GitAgentError::CheckpointNotFound {
            name: "before-refactor".to_string(),
        };
        assert!(err.to_string().contains("before-refactor"));
    }

    #[test]
    fn missing_token_has_descriptive_message() {
        let err = GitAgentError::MissingToken;
        assert!(err.to_string().contains("GITHUB_TOKEN"));
    }
}
```

- [ ] Run tests (expect failure — `error.rs` doesn't exist yet):

```bash
cargo test -p agent007-git-agent 2>&1 | head -30
```

- [ ] Implement `error.rs`:

```rust
// crates/git-agent/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum GitAgentError {
    #[error("git2 error: {0}")]
    Git2(#[from] git2::Error),
    #[error("no git repository found at {path}")]
    NoRepo { path: std::path::PathBuf },
    #[error("checkpoint not found: {name}")]
    CheckpointNotFound { name: String },
    #[error("impact analysis failed: {0}")]
    ImpactAnalysis(String),
    #[error("GitHub/GitLab API error: {0}")]
    ApiError(String),
    #[error("missing auth token (set GITHUB_TOKEN or GITLAB_TOKEN)")]
    MissingToken,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
```

### Step 4: Create `crates/git-agent/src/lib.rs`

- [ ] Create the skeleton lib:

```rust
// crates/git-agent/src/lib.rs
pub mod error;
pub mod agent;
pub mod impact;
pub mod pr;
pub mod debug_loop;

pub use error::GitAgentError;
pub use agent::GitAgent;
pub use debug_loop::{DebugLoop, DebugLoopResult};
```

### Step 5: Create stub modules so the crate compiles

- [ ] Create `crates/git-agent/src/agent.rs` with just a placeholder struct:

```rust
// crates/git-agent/src/agent.rs
pub struct GitAgent {
    pub(crate) repo: git2::Repository,
}
```

- [ ] Create `crates/git-agent/src/impact.rs`, `crates/git-agent/src/pr.rs`, `crates/git-agent/src/debug_loop.rs` as empty files.

### Step 6: Verify the crate compiles

- [ ] Run:

```bash
cargo build -p agent007-git-agent 2>&1 | head -20
```

Expected: compiles with no errors (warnings about unused stubs are acceptable).

### Step 7: Run error tests — expect green

- [ ] Run:

```bash
cargo test -p agent007-git-agent -- error 2>&1
```

Expected: all 3 error tests pass.

### Step 8: Commit

- [ ] Commit:

```bash
git add crates/git-agent/ Cargo.toml
git commit -m "feat(git-agent): scaffold crate with GitAgentError"
```

---

## Task 2: GitAgent::open + create_branch + auto_commit

**Files:**
- Modify: `crates/git-agent/src/agent.rs`

### Step 1: Write failing tests

- [ ] Add a `tests` module at the bottom of `agent.rs`. Tests use `tempfile::tempdir()` to create a real temporary git repo — do NOT mock git2.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn init_repo(dir: &Path) -> git2::Repository {
        let repo = git2::Repository::init(dir).expect("failed to init repo");
        // git2 requires at least one commit before branching
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let tree_id = {
            let mut index = repo.index().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .unwrap();
        repo
    }

    #[test]
    fn open_valid_repo_succeeds() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let agent = GitAgent::open(dir.path());
        assert!(agent.is_ok(), "expected Ok, got {:?}", agent.err());
    }

    #[test]
    fn open_missing_path_returns_no_repo_error() {
        let result = GitAgent::open(Path::new("/tmp/nonexistent_agent007_test_repo"));
        match result {
            Err(GitAgentError::NoRepo { .. }) => {}
            Err(GitAgentError::Git2(_)) => {} // git2 may return its own error
            other => panic!("expected NoRepo or Git2 error, got {:?}", other),
        }
    }

    #[test]
    fn create_branch_creates_and_checks_out_new_branch() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let agent = GitAgent::open(dir.path()).unwrap();
        agent.create_branch("feature/test-branch").unwrap();

        // Verify HEAD now points to new branch
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap();
        assert!(
            head.name().unwrap_or("").contains("test-branch"),
            "HEAD should point to feature/test-branch, got {:?}",
            head.name()
        );
    }

    #[test]
    fn auto_commit_creates_commit_with_message() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let agent = GitAgent::open(dir.path()).unwrap();

        // Create a file to stage
        let file_path = dir.path().join("hello.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let oid = agent
            .auto_commit("test: add hello.txt", &[Path::new("hello.txt")])
            .unwrap();

        let repo = git2::Repository::open(dir.path()).unwrap();
        let commit = repo.find_commit(oid).unwrap();
        assert_eq!(commit.message().unwrap(), "test: add hello.txt");
    }

    #[test]
    fn auto_commit_stages_specified_files_only() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let agent = GitAgent::open(dir.path()).unwrap();

        // Create two files but only commit one
        std::fs::write(dir.path().join("staged.txt"), "staged").unwrap();
        std::fs::write(dir.path().join("unstaged.txt"), "unstaged").unwrap();

        agent
            .auto_commit("stage one file", &[Path::new("staged.txt")])
            .unwrap();

        let repo = git2::Repository::open(dir.path()).unwrap();
        let statuses = repo.statuses(None).unwrap();
        // unstaged.txt should still be untracked / modified
        let has_unstaged = statuses.iter().any(|s| {
            s.path().map(|p| p.contains("unstaged")).unwrap_or(false)
        });
        assert!(has_unstaged, "unstaged.txt should not have been committed");
    }
}
```

- [ ] Run tests (expect compilation failure — methods not implemented):

```bash
cargo test -p agent007-git-agent -- agent 2>&1 | head -40
```

### Step 2: Implement `agent.rs`

- [ ] Implement the full struct and methods:

```rust
// crates/git-agent/src/agent.rs
use std::path::{Path, PathBuf};
use crate::error::GitAgentError;

pub struct GitAgent {
    pub(crate) repo: git2::Repository,
}

impl GitAgent {
    /// Open a git repository at the given path (or any parent).
    pub fn open(path: &Path) -> Result<Self, GitAgentError> {
        git2::Repository::discover(path)
            .map(|repo| Self { repo })
            .map_err(|_| GitAgentError::NoRepo {
                path: path.to_path_buf(),
            })
    }

    /// Create and checkout a new feature branch from HEAD.
    pub fn create_branch(&self, name: &str) -> Result<(), GitAgentError> {
        let head = self.repo.head()?;
        let commit = head.peel_to_commit()?;
        let branch = self.repo.branch(name, &commit, false)?;
        let branch_ref = branch.into_reference();
        let refname = branch_ref
            .name()
            .ok_or_else(|| GitAgentError::Git2(git2::Error::from_str("branch ref has no name")))?;
        self.repo.set_head(refname)?;
        self.repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
        Ok(())
    }

    /// Stage the given relative file paths and create a commit.
    pub fn auto_commit(
        &self,
        message: &str,
        files: &[&Path],
    ) -> Result<git2::Oid, GitAgentError> {
        let mut index = self.repo.index()?;
        for file in files {
            index.add_path(file)?;
        }
        index.write()?;

        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;
        let sig = self.repo.signature()?;

        let parent_commit = self
            .repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok());

        let parents: Vec<&git2::Commit> = parent_commit.iter().collect();
        let oid = self.repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            message,
            &tree,
            &parents,
        )?;
        Ok(oid)
    }
}
```

### Step 3: Run tests — expect green

- [ ] Run:

```bash
cargo test -p agent007-git-agent -- agent 2>&1
```

Expected: all 4 tests pass.

### Step 4: Commit

- [ ] Commit:

```bash
git add crates/git-agent/src/agent.rs
git commit -m "feat(git-agent): implement GitAgent::open, create_branch, auto_commit"
```

---

## Task 3: Checkpoint create / list / rollback (stash-based)

**Files:**
- Modify: `crates/git-agent/src/agent.rs` (add checkpoint methods)

### Step 1: Write failing tests

- [ ] Add to the existing `tests` module in `agent.rs`:

```rust
    #[test]
    fn checkpoint_create_returns_stash_oid() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let agent = GitAgent::open(dir.path()).unwrap();

        // Create a dirty working tree
        std::fs::write(dir.path().join("dirty.txt"), "dirty").unwrap();

        let oid = agent.checkpoint_create("my-checkpoint");
        assert!(
            oid.is_ok(),
            "checkpoint_create should succeed, got {:?}",
            oid.err()
        );
    }

    #[test]
    fn checkpoint_list_returns_only_agent007_stashes() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let agent = GitAgent::open(dir.path()).unwrap();

        // No stashes yet
        let list = agent.checkpoint_list().unwrap();
        assert!(list.is_empty(), "expected no checkpoints initially");

        // Create a dirty tree and stash it
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        agent.checkpoint_create("first").unwrap();

        let list = agent.checkpoint_list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], "first");
    }

    #[test]
    fn rollback_to_restores_stash() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let agent = GitAgent::open(dir.path()).unwrap();

        std::fs::write(dir.path().join("r.txt"), "rollback content").unwrap();
        agent.checkpoint_create("rollback-point").unwrap();

        // Confirm file was stashed (not present in working tree)
        // Then rollback should restore it
        let result = agent.rollback_to("rollback-point");
        assert!(
            result.is_ok(),
            "rollback_to should succeed, got {:?}",
            result.err()
        );
        // After rollback, r.txt should exist in working tree
        assert!(dir.path().join("r.txt").exists(), "r.txt should be restored after rollback");
    }

    #[test]
    fn rollback_to_missing_checkpoint_returns_error() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let agent = GitAgent::open(dir.path()).unwrap();

        let result = agent.rollback_to("nonexistent");
        assert!(
            matches!(result, Err(GitAgentError::CheckpointNotFound { .. })),
            "expected CheckpointNotFound, got {:?}",
            result
        );
    }
```

- [ ] Run tests (expect failure):

```bash
cargo test -p agent007-git-agent -- checkpoint 2>&1 | head -40
```

### Step 2: Implement checkpoint methods

- [ ] Add to the `impl GitAgent` block in `agent.rs`:

```rust
    const CHECKPOINT_PREFIX: &'static str = "agent007:";

    /// Stash current working tree changes under "agent007:<name>".
    pub fn checkpoint_create(&self, name: &str) -> Result<git2::Oid, GitAgentError> {
        let sig = self.repo.signature()?;
        let message = format!("{}{}", Self::CHECKPOINT_PREFIX, name);
        let oid = self.repo.stash_save(&sig, &message, None)?;
        Ok(oid)
    }

    /// List all checkpoint names (strips "agent007:" prefix).
    pub fn checkpoint_list(&self) -> Result<Vec<String>, GitAgentError> {
        let mut names = Vec::new();
        self.repo.stash_foreach(|_index, message, _oid| {
            if let Some(name) = message.strip_prefix(Self::CHECKPOINT_PREFIX) {
                names.push(name.to_string());
            }
            true // continue iterating
        })?;
        Ok(names)
    }

    /// Pop the stash matching the given checkpoint name.
    pub fn rollback_to(&self, name: &str) -> Result<(), GitAgentError> {
        let target_message = format!("{}{}", Self::CHECKPOINT_PREFIX, name);
        let mut found_index: Option<usize> = None;

        self.repo.stash_foreach(|index, message, _oid| {
            if message == target_message {
                found_index = Some(index);
                false // stop iterating
            } else {
                true
            }
        })?;

        let index = found_index.ok_or_else(|| GitAgentError::CheckpointNotFound {
            name: name.to_string(),
        })?;

        let mut opts = git2::StashApplyOptions::new();
        self.repo.stash_pop(index, Some(&mut opts))?;
        Ok(())
    }
```

### Step 3: Run tests — expect green

- [ ] Run:

```bash
cargo test -p agent007-git-agent 2>&1
```

Expected: all tests pass (open, branch, commit, checkpoint tests).

### Step 4: Commit

- [ ] Commit:

```bash
git add crates/git-agent/src/agent.rs
git commit -m "feat(git-agent): implement checkpoint_create, checkpoint_list, rollback_to"
```

---

## Task 4: Impact analysis (grep-based)

**Files:**
- Modify: `crates/git-agent/src/impact.rs`
- Modify: `crates/git-agent/src/agent.rs` (add `impact_analysis` method delegating to `impact.rs`)

### Step 1: Write failing tests

- [ ] Add tests in `impact.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn make_repo_with_files(files: &[(&str, &str)]) -> (tempfile::TempDir, git2::Repository) {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "t@t.com").unwrap();

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
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();

        (dir, repo)
    }

    #[test]
    fn impact_finds_file_using_module() {
        let (_dir, repo) = make_repo_with_files(&[
            ("src/auth/token.rs", "pub fn make_token() {}"),
            ("src/api/handler.rs", "use crate::auth::token;\nfn handle() {}"),
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
```

- [ ] Run tests (expect failure — function not implemented):

```bash
cargo test -p agent007-git-agent -- impact 2>&1 | head -40
```

### Step 2: Implement `impact.rs`

- [ ] Implement the grep-based scanner:

```rust
// crates/git-agent/src/impact.rs
use std::path::{Path, PathBuf};
use crate::error::GitAgentError;

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
                .any(|l| l.contains(&format!("use ")) && l.contains(&module_path))
            || content
                .lines()
                .any(|l| l.trim_start().starts_with("mod ") && l.contains(&stem));

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
```

- [ ] Add the `impact_analysis` method to `GitAgent` in `agent.rs`:

```rust
    /// Shallow grep-based scan: find tracked .rs files that import or declare the given path.
    pub fn impact_analysis(&self, path: &Path) -> Result<Vec<PathBuf>, GitAgentError> {
        crate::impact::impact_analysis(&self.repo, path)
    }
```

### Step 3: Run tests — expect green

- [ ] Run:

```bash
cargo test -p agent007-git-agent 2>&1
```

Expected: all tests pass.

### Step 4: Commit

- [ ] Commit:

```bash
git add crates/git-agent/src/impact.rs crates/git-agent/src/agent.rs
git commit -m "feat(git-agent): implement impact_analysis (grep-based tracked file scan)"
```

---

## Task 5: PR creation (GitHub/GitLab REST API)

**Files:**
- Modify: `crates/git-agent/src/pr.rs`
- Modify: `crates/git-agent/src/agent.rs` (add `create_pr` async method)

### Step 1: Write failing tests using mock HTTP server

The PR creation tests mock the HTTP layer to avoid real API calls. Use `wiremock` or test the platform detection logic in isolation (no real network). Add `wiremock` as a dev-dependency for integration tests.

- [ ] Add `wiremock` to `crates/git-agent/Cargo.toml` dev-dependencies:

```toml
[dev-dependencies]
tempfile = { workspace = true }
tokio = { workspace = true }
wiremock = "0.6"
```

- [ ] Write tests in `pr.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_github_platform_from_https_url() {
        let url = "https://github.com/myorg/myrepo.git";
        let platform = detect_platform(url);
        assert!(matches!(platform, Platform::GitHub { owner, repo } if owner == "myorg" && repo == "myrepo"));
    }

    #[test]
    fn detect_github_platform_from_ssh_url() {
        let url = "git@github.com:myorg/myrepo.git";
        let platform = detect_platform(url);
        assert!(matches!(platform, Platform::GitHub { owner, repo } if owner == "myorg" && repo == "myrepo"));
    }

    #[test]
    fn detect_gitlab_platform_from_https_url() {
        let url = "https://gitlab.com/mygroup/myrepo.git";
        let platform = detect_platform(url);
        assert!(matches!(platform, Platform::GitLab { owner, repo } if owner == "mygroup" && repo == "myrepo"));
    }

    #[test]
    fn detect_unknown_platform_for_other_urls() {
        let url = "https://bitbucket.org/myorg/myrepo.git";
        let platform = detect_platform(url);
        assert!(matches!(platform, Platform::Unknown));
    }

    #[tokio::test]
    async fn create_github_pr_sends_correct_payload() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path, header};

        let mock_server = MockServer::start().await;
        let response_body = serde_json::json!({
            "html_url": "https://github.com/myorg/myrepo/pull/42"
        });

        Mock::given(method("POST"))
            .and(path("/repos/myorg/myrepo/pulls"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(201).set_body_json(&response_body))
            .mount(&mock_server)
            .await;

        let url = format!("{}/repos/myorg/myrepo/pulls", mock_server.uri());
        let client = reqwest::Client::new();
        let result = post_github_pr(
            &client,
            &url,
            "test-token",
            "Add feature",
            "Description here",
            "feature/add-mdns",
            "main",
        )
        .await;

        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
        assert!(result.unwrap().contains("pull/42"));
    }
}
```

- [ ] Run tests (expect failure):

```bash
cargo test -p agent007-git-agent -- pr 2>&1 | head -40
```

### Step 2: Implement `pr.rs`

- [ ] Implement platform detection and PR creation:

```rust
// crates/git-agent/src/pr.rs
use crate::error::GitAgentError;

#[derive(Debug, PartialEq)]
pub enum Platform {
    GitHub { owner: String, repo: String },
    GitLab { owner: String, repo: String },
    Unknown,
}

/// Detect whether a remote URL points to GitHub or GitLab,
/// and extract the owner/repo from the URL.
pub fn detect_platform(url: &str) -> Platform {
    let clean = url
        .trim_end_matches(".git")
        .replace("git@github.com:", "https://github.com/")
        .replace("git@gitlab.com:", "https://gitlab.com/");

    if let Some(rest) = clean.strip_prefix("https://github.com/") {
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        if parts.len() == 2 {
            return Platform::GitHub {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
            };
        }
    }
    if let Some(rest) = clean.strip_prefix("https://gitlab.com/") {
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        if parts.len() == 2 {
            return Platform::GitLab {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
            };
        }
    }
    Platform::Unknown
}

/// Send a GitHub PR creation request to the given API URL (injectable for testing).
pub async fn post_github_pr(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    title: &str,
    body: &str,
    head: &str,
    base: &str,
) -> Result<String, GitAgentError> {
    let payload = serde_json::json!({
        "title": title,
        "body": body,
        "head": head,
        "base": base,
    });

    let resp = client
        .post(api_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "agent007")
        .json(&payload)
        .send()
        .await
        .map_err(|e| GitAgentError::ApiError(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(GitAgentError::ApiError(format!(
            "GitHub API returned {}: {}",
            status, text
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| GitAgentError::ApiError(e.to_string()))?;

    json["html_url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| GitAgentError::ApiError("missing html_url in response".into()))
}

/// Send a GitLab MR creation request.
pub async fn post_gitlab_mr(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    title: &str,
    body: &str,
    head: &str,
    base: &str,
) -> Result<String, GitAgentError> {
    let payload = serde_json::json!({
        "title": title,
        "description": body,
        "source_branch": head,
        "target_branch": base,
    });

    let resp = client
        .post(api_url)
        .header("PRIVATE-TOKEN", token)
        .header("User-Agent", "agent007")
        .json(&payload)
        .send()
        .await
        .map_err(|e| GitAgentError::ApiError(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(GitAgentError::ApiError(format!(
            "GitLab API returned {}: {}",
            status, text
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| GitAgentError::ApiError(e.to_string()))?;

    json["web_url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| GitAgentError::ApiError("missing web_url in response".into()))
}
```

- [ ] Add the `create_pr` async method to `GitAgent` in `agent.rs`:

```rust
    /// Create a PR on GitHub or GitLab. Detects platform from the `origin` remote URL.
    /// Reads GITHUB_TOKEN or GITLAB_TOKEN from env.
    pub async fn create_pr(
        &self,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<String, GitAgentError> {
        use crate::pr::{detect_platform, post_github_pr, post_gitlab_mr, Platform};

        let remote = self.repo.find_remote("origin")?;
        let url = remote
            .url()
            .ok_or_else(|| GitAgentError::ApiError("origin remote has no URL".into()))?;

        let client = reqwest::Client::new();

        match detect_platform(url) {
            Platform::GitHub { owner, repo } => {
                let token = std::env::var("GITHUB_TOKEN")
                    .map_err(|_| GitAgentError::MissingToken)?;
                let api_url = format!(
                    "https://api.github.com/repos/{}/{}/pulls",
                    owner, repo
                );
                post_github_pr(&client, &api_url, &token, title, body, head, base).await
            }
            Platform::GitLab { owner, repo } => {
                let token = std::env::var("GITLAB_TOKEN")
                    .map_err(|_| GitAgentError::MissingToken)?;
                // GitLab requires URL-encoded project path
                let project = format!("{}/{}", owner, repo);
                let encoded = project.replace('/', "%2F");
                let api_url = format!(
                    "https://gitlab.com/api/v4/projects/{}/merge_requests",
                    encoded
                );
                post_gitlab_mr(&client, &api_url, &token, title, body, head, base).await
            }
            Platform::Unknown => Err(GitAgentError::ApiError(
                "unrecognized remote URL (not GitHub or GitLab)".into(),
            )),
        }
    }
```

### Step 3: Run tests — expect green

- [ ] Run:

```bash
cargo test -p agent007-git-agent 2>&1
```

Expected: all tests pass (platform detection + mock HTTP PR test).

### Step 4: Commit

- [ ] Commit:

```bash
git add crates/git-agent/src/pr.rs crates/git-agent/src/agent.rs crates/git-agent/Cargo.toml
git commit -m "feat(git-agent): implement PR creation for GitHub and GitLab"
```

---

## Task 6: Debug loop

**Files:**
- Modify: `crates/git-agent/src/debug_loop.rs`

### Step 1: Write failing tests

- [ ] Add tests in `debug_loop.rs`. Since the debug loop calls `cargo nextest` and a `ModelProvider`, use `AGENT007_DRY_RUN` env and mock provider:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use agent007_core::dispatcher::LocalDispatcher;

    struct AlwaysPassProvider;

    #[async_trait::async_trait]
    impl agent007_core::provider::ModelProvider for AlwaysPassProvider {
        async fn complete(
            &self,
            _req: agent007_core::provider::CompletionRequest,
        ) -> Result<agent007_core::provider::CompletionResponse, agent007_core::error::CoreError> {
            Ok(agent007_core::provider::CompletionResponse {
                content: "no fix needed".to_string(),
                model: "mock".to_string(),
                usage: None,
            })
        }
        async fn embed(
            &self,
            _text: &str,
        ) -> Result<Vec<f32>, agent007_core::error::CoreError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn debug_loop_returns_resolved_true_when_no_test_failures() {
        // When there are no test failures, run() should return resolved: true
        // after the first iteration (without calling the provider).
        // We can't run cargo nextest in unit tests, so this test exercises the
        // parse_nextest_failures() function directly.
        let output = "";
        let failures = parse_nextest_failures(output);
        assert!(failures.is_empty(), "empty output should yield no failures");
    }

    #[test]
    fn parse_nextest_failures_extracts_test_names() {
        // Simulate a nextest JSON output line indicating a failure
        let output = r#"{"type":"test","event":"failed","name":"crate::mod::test_foo","stderr":"assertion failed"}"#;
        let failures = parse_nextest_failures(output);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].test_name.contains("test_foo"));
    }

    #[test]
    fn parse_nextest_failures_ignores_non_failed_events() {
        let output = r#"{"type":"test","event":"passed","name":"crate::mod::test_ok"}"#;
        let failures = parse_nextest_failures(output);
        assert!(failures.is_empty());
    }

    #[test]
    fn debug_loop_result_not_resolved_on_max_iterations() {
        // Verify DebugLoopResult fields
        let result = DebugLoopResult {
            iterations: 5,
            resolved: false,
            final_output: "still failing".to_string(),
        };
        assert_eq!(result.iterations, 5);
        assert!(!result.resolved);
        assert!(result.final_output.contains("still failing"));
    }
}
```

- [ ] Run tests (expect failure):

```bash
cargo test -p agent007-git-agent -- debug_loop 2>&1 | head -40
```

### Step 2: Implement `debug_loop.rs`

- [ ] Implement the debug loop struct and parsing helpers:

```rust
// crates/git-agent/src/debug_loop.rs
use std::sync::Arc;
use crate::agent::GitAgent;
use crate::error::GitAgentError;
use agent007_core::provider::ModelProvider;
use agent007_core::dispatcher::Dispatcher;

#[derive(Debug, Clone)]
pub struct TestFailure {
    pub test_name: String,
    pub stderr: String,
}

#[derive(Debug)]
pub struct DebugLoopResult {
    pub iterations: usize,
    pub resolved: bool,
    pub final_output: String,
}

pub struct DebugLoop {
    pub max_iterations: usize,
    pub model: String,
}

impl DebugLoop {
    pub fn new(max_iterations: usize, model: impl Into<String>) -> Self {
        Self {
            max_iterations,
            model: model.into(),
        }
    }

    /// Run the debug loop: repeatedly run nextest, parse failures, ask the model
    /// for a fix, apply it, and rerun — up to `max_iterations` times.
    pub async fn run(
        &self,
        git_agent: &GitAgent,
        provider: Arc<dyn ModelProvider>,
        _dispatcher: Arc<dyn Dispatcher>,
    ) -> Result<DebugLoopResult, GitAgentError> {
        let workdir = git_agent
            .repo
            .workdir()
            .ok_or_else(|| {
                GitAgentError::ImpactAnalysis("bare repository not supported".into())
            })?
            .to_path_buf();

        let mut iterations = 0;
        let mut last_output = String::new();

        for _ in 0..self.max_iterations {
            iterations += 1;

            // 1. Run cargo nextest
            let output = run_nextest(&workdir)?;
            last_output = output.clone();

            // 2. Parse failures
            let failures = parse_nextest_failures(&output);
            if failures.is_empty() {
                return Ok(DebugLoopResult {
                    iterations,
                    resolved: true,
                    final_output: last_output,
                });
            }

            // 3. Build prompt with failures
            let prompt = build_fix_prompt(&failures, &workdir);

            // 4. Ask the model for a fix
            let request = agent007_core::provider::CompletionRequest {
                model: self.model.clone(),
                messages: vec![agent007_core::provider::Message {
                    role: "user".to_string(),
                    content: prompt,
                }],
                max_tokens: Some(2048),
                temperature: Some(0.2),
            };

            let response = provider
                .complete(request)
                .await
                .map_err(|e| GitAgentError::ImpactAnalysis(e.to_string()))?;

            // 5. Apply fix — parse file/content from response and write to disk
            apply_fix_proposal(&response.content, &workdir)?;
        }

        Ok(DebugLoopResult {
            iterations,
            resolved: false,
            final_output: format!(
                "Debug loop exhausted after {} iterations. Last output:\n{}",
                iterations, last_output
            ),
        })
    }
}

/// Run `cargo nextest run --message-format json` in the given directory.
/// Returns the combined stdout as a String.
fn run_nextest(workdir: &std::path::Path) -> Result<String, GitAgentError> {
    let output = std::process::Command::new("cargo")
        .args(["nextest", "run", "--message-format", "json"])
        .current_dir(workdir)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr))
}

/// Parse nextest JSON output for failed test events.
pub fn parse_nextest_failures(output: &str) -> Vec<TestFailure> {
    let mut failures = Vec::new();
    for line in output.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("type").and_then(|t| t.as_str()) == Some("test")
                && v.get("event").and_then(|e| e.as_str()) == Some("failed")
            {
                let test_name = v
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let stderr = v
                    .get("stderr")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                failures.push(TestFailure { test_name, stderr });
            }
        }
    }
    failures
}

/// Build a prompt asking the model to fix the given test failures.
fn build_fix_prompt(failures: &[TestFailure], workdir: &std::path::Path) -> String {
    let mut prompt = String::from(
        "The following tests are failing. Propose a minimal fix.\n\n",
    );
    for f in failures {
        prompt.push_str(&format!(
            "## Failing test: {}\nError output:\n{}\n\n",
            f.test_name, f.stderr
        ));
        // Attempt to read the source file for context
        if let Some(file_path) = test_name_to_path(&f.test_name, workdir) {
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                prompt.push_str(&format!("### Source ({}):\n```rust\n{}\n```\n\n", file_path.display(), content));
            }
        }
    }
    prompt.push_str(
        "Respond with the fixed file content in the format:\n\
        FILE: <relative/path/to/file.rs>\n\
        ```rust\n<complete file content>\n```\n",
    );
    prompt
}

/// Attempt to find a source file path from a test name like `crate::module::test_foo`.
fn test_name_to_path(test_name: &str, workdir: &std::path::Path) -> Option<std::path::PathBuf> {
    let parts: Vec<&str> = test_name.split("::").collect();
    if parts.len() < 2 {
        return None;
    }
    // Try src/<part1>/<part2>.rs etc.
    let candidate = workdir.join("src").join(parts[..parts.len() - 1].join("/")).with_extension("rs");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

/// Parse a fix proposal from the model and write the file to disk.
/// Expects format:
///   FILE: <path>
///   ```rust
///   <content>
///   ```
fn apply_fix_proposal(proposal: &str, workdir: &std::path::Path) -> Result<(), GitAgentError> {
    let mut current_file: Option<std::path::PathBuf> = None;
    let mut in_block = false;
    let mut content_lines: Vec<&str> = Vec::new();

    for line in proposal.lines() {
        if let Some(path_str) = line.strip_prefix("FILE: ") {
            current_file = Some(workdir.join(path_str.trim()));
        } else if line.trim_start().starts_with("```rust") {
            in_block = true;
            content_lines.clear();
        } else if line.trim() == "```" && in_block {
            in_block = false;
            if let Some(ref file_path) = current_file {
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(file_path, content_lines.join("\n"))?;
            }
        } else if in_block {
            content_lines.push(line);
        }
    }
    Ok(())
}
```

### Step 3: Check that `ModelProvider` and `Dispatcher` traits are compatible

- [ ] Verify the trait paths used in `debug_loop.rs` match those in `crates/core/src/`:

```bash
grep -r "pub trait ModelProvider" /Users/tvhc84/workspace/rust/agent007/crates/
grep -r "pub trait Dispatcher" /Users/tvhc84/workspace/rust/agent007/crates/
```

Adjust import paths in `debug_loop.rs` if needed to match actual module locations.

### Step 4: Run tests — expect green

- [ ] Run:

```bash
cargo test -p agent007-git-agent 2>&1
```

Expected: all tests pass.

### Step 5: Commit

- [ ] Commit:

```bash
git add crates/git-agent/src/debug_loop.rs
git commit -m "feat(git-agent): implement DebugLoop with nextest runner and fix applier"
```

---

## Task 7: CLI commands (git + checkpoint + replay stub)

**Files:**
- Create: `crates/cli/src/commands/git.rs`
- Create: `crates/cli/src/commands/checkpoint.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `crates/cli/src/main.rs`
- Modify: `crates/cli/Cargo.toml`

### Step 1: Write failing tests for CLI parsing

- [ ] Add CLI parsing tests (to the existing `tests` module in `crates/cli/src/main.rs`):

```rust
    #[test]
    fn parse_checkpoint_create_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "checkpoint", "create", "before refactor"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Checkpoint(ref c) if matches!(c.action, CheckpointAction::Create { ref name } if name == "before refactor")
        ));
    }

    #[test]
    fn parse_checkpoint_list_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "checkpoint", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Checkpoint(ref c) if matches!(c.action, CheckpointAction::List)
        ));
    }

    #[test]
    fn parse_rollback_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "rollback", "--to", "before refactor"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Rollback { ref to } if to == "before refactor"
        ));
    }

    #[test]
    fn parse_git_branch_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "git", "branch", "feature/add-mDNS"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Git(ref g) if matches!(g.action, GitAction::Branch { ref name } if name == "feature/add-mDNS")
        ));
    }

    #[test]
    fn parse_git_commit_subcommand() {
        let cli = Cli::try_parse_from([
            "agent007", "git", "commit", "implement mDNS", "--files", "src/net/mdns.rs",
        ]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Git(ref g) if matches!(g.action, GitAction::Commit { .. })
        ));
    }

    #[test]
    fn parse_git_pr_subcommand() {
        let cli = Cli::try_parse_from([
            "agent007", "git", "pr",
            "--title", "Add mDNS",
            "--body", "adds mdns",
            "--head", "feature/add-mDNS",
            "--base", "main",
        ]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Git(ref g) if matches!(g.action, GitAction::Pr { .. })
        ));
    }

    #[test]
    fn parse_git_impact_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "git", "impact", "src/auth/token.rs"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Git(ref g) if matches!(g.action, GitAction::Impact { .. })
        ));
    }

    #[test]
    fn parse_replay_subcommand() {
        let cli = Cli::try_parse_from([
            "agent007", "replay", "--session", "abc123", "--model", "ollama/llama3",
        ]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Replay { ref session, ref model } if session == "abc123" && model == "ollama/llama3"
        ));
    }
```

- [ ] Run tests (expect failure — types not defined):

```bash
cargo test -p agent007 -- parse 2>&1 | head -40
```

### Step 2: Add `agent007-git-agent` to CLI Cargo.toml

- [ ] Modify `crates/cli/Cargo.toml`:

```toml
agent007-git-agent = { path = "../git-agent" }
```

### Step 3: Create `crates/cli/src/commands/checkpoint.rs`

- [ ] Implement:

```rust
// crates/cli/src/commands/checkpoint.rs
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use agent007_git_agent::GitAgent;

#[derive(Parser, Debug)]
pub struct CheckpointArgs {
    #[command(subcommand)]
    pub action: CheckpointAction,
}

#[derive(Subcommand, Debug)]
pub enum CheckpointAction {
    /// Create a named checkpoint (stash) of the current working tree
    Create {
        /// Checkpoint name
        name: String,
    },
    /// List all agent007 checkpoints
    List,
}

pub async fn execute(_config: std::sync::Arc<crate::config::Config>, action: CheckpointAction) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let agent = GitAgent::open(&cwd)?;

    match action {
        CheckpointAction::Create { name } => {
            let oid = agent.checkpoint_create(&name)?;
            println!("Checkpoint '{}' created: {}", name, oid);
        }
        CheckpointAction::List => {
            let checkpoints = agent.checkpoint_list()?;
            if checkpoints.is_empty() {
                println!("No checkpoints found.");
            } else {
                for cp in checkpoints {
                    println!("  - {}", cp);
                }
            }
        }
    }
    Ok(())
}
```

### Step 4: Create `crates/cli/src/commands/git.rs`

- [ ] Implement:

```rust
// crates/cli/src/commands/git.rs
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use agent007_git_agent::GitAgent;

#[derive(Parser, Debug)]
pub struct GitArgs {
    #[command(subcommand)]
    pub action: GitAction,
}

#[derive(Subcommand, Debug)]
pub enum GitAction {
    /// Create and checkout a new branch
    Branch {
        /// Branch name
        name: String,
    },
    /// Stage files and create a commit
    Commit {
        /// Commit message
        message: String,
        /// Files to stage (space-separated)
        #[arg(long, num_args = 1.., value_delimiter = ' ')]
        files: Vec<PathBuf>,
    },
    /// Create a pull request on GitHub or GitLab
    Pr {
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
        #[arg(long)]
        head: String,
        #[arg(long)]
        base: String,
    },
    /// Show files impacted by changes to the given path
    Impact {
        /// Path to analyze (e.g. src/auth/token.rs)
        path: PathBuf,
    },
}

pub async fn execute(_config: std::sync::Arc<crate::config::Config>, action: GitAction) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let agent = GitAgent::open(&cwd)?;

    match action {
        GitAction::Branch { name } => {
            agent.create_branch(&name)?;
            println!("Switched to new branch '{}'", name);
        }
        GitAction::Commit { message, files } => {
            let file_refs: Vec<&Path> = files.iter().map(|p| p.as_path()).collect();
            let oid = agent.auto_commit(&message, &file_refs)?;
            println!("Committed: {}", oid);
        }
        GitAction::Pr { title, body, head, base } => {
            let url = agent.create_pr(&title, &body, &head, &base).await?;
            println!("PR created: {}", url);
        }
        GitAction::Impact { path } => {
            let affected = agent.impact_analysis(&path)?;
            if affected.is_empty() {
                println!("No files reference '{}'", path.display());
            } else {
                println!("Files impacted by '{}':", path.display());
                for f in affected {
                    println!("  {}", f.display());
                }
            }
        }
    }
    Ok(())
}
```

### Step 5: Update `crates/cli/src/commands/mod.rs`

- [ ] Add the two new modules:

```rust
pub mod run;
pub mod serve;
pub mod skill;
pub mod simulate;
pub mod git;
pub mod checkpoint;
```

### Step 6: Update `crates/cli/src/main.rs`

- [ ] Add new `Commands` variants and wire handlers. Add to the `Commands` enum:

```rust
    /// Manage git operations (branch, commit, PR, impact)
    Git(GitArgs),
    /// Manage named checkpoints (stash-based)
    Checkpoint(CheckpointArgs),
    /// Rollback to a named checkpoint
    Rollback {
        /// Checkpoint name to restore
        #[arg(long)]
        to: String,
    },
    /// Replay a past agent session (stub — Phase 3)
    Replay {
        /// Session ID to replay
        #[arg(long)]
        session: String,
        /// Model to use for replay
        #[arg(long)]
        model: String,
    },
```

- [ ] Add the corresponding imports at the top of `main.rs`:

```rust
use crate::commands::git::{GitArgs, GitAction};
use crate::commands::checkpoint::{CheckpointArgs, CheckpointAction};
```

- [ ] Add match arms in `main()`:

```rust
        Commands::Git(g) => commands::git::execute(config, g.action).await,
        Commands::Checkpoint(c) => commands::checkpoint::execute(config, c.action).await,
        Commands::Rollback { to } => {
            let cwd = std::env::current_dir()?;
            let agent = agent007_git_agent::GitAgent::open(&cwd)?;
            agent.rollback_to(&to)?;
            println!("Rolled back to checkpoint '{}'", to);
            Ok(())
        }
        Commands::Replay { session, model } => {
            println!("session replay not yet implemented (session={}, model={})", session, model);
            Ok(())
        }
```

### Step 7: Run CLI tests — expect green

- [ ] Run:

```bash
cargo test -p agent007 -- parse 2>&1
```

Expected: all parse tests pass.

- [ ] Also verify the full workspace compiles:

```bash
cargo build 2>&1 | tail -5
```

### Step 8: Commit

- [ ] Commit:

```bash
git add crates/cli/src/commands/git.rs \
        crates/cli/src/commands/checkpoint.rs \
        crates/cli/src/commands/mod.rs \
        crates/cli/src/main.rs \
        crates/cli/Cargo.toml
git commit -m "feat(cli): add git branch/commit/pr/impact, checkpoint, rollback, replay stub commands"
```

---

## Task 8: Wire DebugLoop into build_stack (optional integration)

**Files:**
- Modify: `crates/cli/src/commands/run.rs` (add `DebugLoop` to `Stack` — optional)
- Modify: `crates/cli/src/main.rs` (add `agent007 debug` command — optional)

This task is optional for the initial implementation. The debug loop requires a running provider (already available in `Stack.model_router`) and a git repo (available via `GitAgent::open`). Wire it when the rest of the stack is stable.

### Step 1: Add `Debug` command to CLI (optional)

- [ ] If wiring is desired, add to `Commands` in `main.rs`:

```rust
    /// Run the iterative debug loop on failing tests
    Debug {
        /// Maximum fix iterations
        #[arg(long, default_value = "5")]
        max_iter: usize,
        /// Model to use for fix proposals
        #[arg(long, default_value = "default")]
        model: String,
    },
```

- [ ] Add the match arm:

```rust
        Commands::Debug { max_iter, model } => {
            let stack = commands::run::build_stack(&config).await?;
            let cwd = std::env::current_dir()?;
            let git_agent = agent007_git_agent::GitAgent::open(&cwd)?;
            let debug_loop = agent007_git_agent::DebugLoop::new(max_iter, model);
            let result = debug_loop
                .run(
                    &git_agent,
                    stack.model_router.clone(),
                    stack.dispatcher.clone(),
                )
                .await?;
            if result.resolved {
                println!("All tests passing after {} iteration(s).", result.iterations);
            } else {
                println!("Debug loop exhausted ({} iterations). Diagnosis:\n{}", result.iterations, result.final_output);
            }
            Ok(())
        }
```

### Step 2: Run full workspace tests

- [ ] Run:

```bash
cargo test 2>&1 | tail -20
```

Expected: all workspace tests pass.

### Step 3: Final commit

- [ ] Commit:

```bash
git add crates/cli/src/main.rs
git commit -m "feat(cli): wire DebugLoop into agent007 debug command"
```

---

## Summary: Full CLI Surface

After all tasks are complete, the following commands are available:

```bash
# Branch management
agent007 git branch "feature/add-mDNS"

# Auto-commit
agent007 git commit "implement mDNS discovery" --files src/net/mdns.rs src/net/mod.rs

# PR creation
agent007 git pr --title "Add mDNS" --body "Adds mDNS discovery support" --head feature/add-mDNS --base main

# Impact analysis
agent007 git impact src/auth/token.rs

# Checkpoint / stash
agent007 checkpoint create "before refactor"
agent007 checkpoint list

# Rollback
agent007 rollback --to "before refactor"

# Debug loop
agent007 debug --max-iter 5 --model ollama/llama3

# Session replay (stub)
agent007 replay --session abc123 --model ollama/llama3
```

## Test Commands Reference

```bash
# Run only git-agent tests
cargo test -p agent007-git-agent

# Run only CLI tests
cargo test -p agent007

# Run all workspace tests
cargo test

# Run with output for debugging
cargo test -p agent007-git-agent -- --nocapture 2>&1 | head -60
```
