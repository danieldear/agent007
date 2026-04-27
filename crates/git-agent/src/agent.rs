// crates/git-agent/src/agent.rs
use crate::error::GitAgentError;
use std::path::{Path, PathBuf};

pub struct GitAgent {
    pub(crate) repo: git2::Repository,
}

impl std::fmt::Debug for GitAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitAgent")
            .field("repo", &self.repo.path())
            .finish()
    }
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
    pub fn auto_commit(&self, message: &str, files: &[&Path]) -> Result<git2::Oid, GitAgentError> {
        let mut index = self.repo.index()?;
        for file in files {
            index.add_path(file)?;
        }
        index.write()?;

        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;
        let sig = self.repo.signature()?;

        let parent_commit = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());

        let parents: Vec<&git2::Commit> = parent_commit.iter().collect();
        let oid = self
            .repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;
        Ok(oid)
    }

    const CHECKPOINT_PREFIX: &'static str = "agent007:";

    /// Stash current working tree changes under "agent007:<name>".
    pub fn checkpoint_create(&mut self, name: &str) -> Result<git2::Oid, GitAgentError> {
        let sig = self.repo.signature()?;
        let message = format!("{}{}", Self::CHECKPOINT_PREFIX, name);
        let flags = git2::StashFlags::INCLUDE_UNTRACKED;
        let oid = self.repo.stash_save(&sig, &message, Some(flags))?;
        Ok(oid)
    }

    /// List all checkpoint names (strips "agent007:" prefix).
    pub fn checkpoint_list(&mut self) -> Result<Vec<String>, GitAgentError> {
        let mut names = Vec::new();
        self.repo.stash_foreach(|_index, message, _oid| {
            // git stash messages are prefixed with "On <branch>: " so we search
            // for the agent007 prefix anywhere in the message
            if let Some(pos) = message.find(Self::CHECKPOINT_PREFIX) {
                let name = &message[pos + Self::CHECKPOINT_PREFIX.len()..];
                names.push(name.to_string());
            }
            true // continue iterating
        })?;
        Ok(names)
    }

    /// Pop the stash matching the given checkpoint name.
    pub fn rollback_to(&mut self, name: &str) -> Result<(), GitAgentError> {
        let target_suffix = format!("{}{}", Self::CHECKPOINT_PREFIX, name);
        let mut found_index: Option<usize> = None;

        self.repo.stash_foreach(|index, message, _oid| {
            if message.contains(&target_suffix) {
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

    /// Shallow grep-based scan: find tracked .rs files that import or declare the given path.
    pub fn impact_analysis(&self, path: &Path) -> Result<Vec<PathBuf>, GitAgentError> {
        crate::impact::impact_analysis(&self.repo, path)
    }

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
                let token =
                    std::env::var("GITHUB_TOKEN").map_err(|_| GitAgentError::MissingToken)?;
                let api_url = format!("https://api.github.com/repos/{}/{}/pulls", owner, repo);
                post_github_pr(&client, &api_url, &token, title, body, head, base).await
            }
            Platform::GitLab { owner, repo } => {
                let token =
                    std::env::var("GITLAB_TOKEN").map_err(|_| GitAgentError::MissingToken)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn init_repo(dir: &Path) -> git2::Repository {
        let repo = git2::Repository::init(dir).expect("failed to init repo");
        // Ensure tests do not depend on global git identity in CI runners.
        {
            let mut cfg = repo.config().expect("failed to open repo config");
            cfg.set_str("user.name", "agent007-tests")
                .expect("failed to set user.name");
            cfg.set_str("user.email", "agent007-tests@example.com")
                .expect("failed to set user.email");
        }
        // git2 requires at least one commit before branching
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let tree_id = {
            let mut index = repo.index().unwrap();
            index.write_tree().unwrap()
        };
        {
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
                .unwrap();
        }
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
        let has_unstaged = statuses
            .iter()
            .any(|s| s.path().map(|p| p.contains("unstaged")).unwrap_or(false));
        assert!(has_unstaged, "unstaged.txt should not have been committed");
    }

    #[test]
    fn checkpoint_create_returns_stash_oid() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let mut agent = GitAgent::open(dir.path()).unwrap();

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
        let mut agent = GitAgent::open(dir.path()).unwrap();

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
        let mut agent = GitAgent::open(dir.path()).unwrap();

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
        assert!(
            dir.path().join("r.txt").exists(),
            "r.txt should be restored after rollback"
        );
    }

    #[test]
    fn rollback_to_missing_checkpoint_returns_error() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let mut agent = GitAgent::open(dir.path()).unwrap();

        let result = agent.rollback_to("nonexistent");
        assert!(
            matches!(result, Err(GitAgentError::CheckpointNotFound { .. })),
            "expected CheckpointNotFound, got {:?}",
            result
        );
    }
}
