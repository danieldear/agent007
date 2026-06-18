use crate::config::Config;
use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser, Debug)]
pub struct ProjectsArgs {
    #[command(subcommand)]
    pub action: ProjectsAction,
}

#[derive(Subcommand, Debug)]
pub enum ProjectsAction {
    /// Register a project in the global agent007 project registry
    Add {
        /// Project directory to register
        path: PathBuf,
        /// Display name to store for the project (defaults to the directory name)
        #[arg(long)]
        name: Option<String>,
        /// Print the updated project entry as JSON
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List projects from the global agent007 project registry
    List {
        /// Print the registry as JSON
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Remove a project by stable id or path
    Remove {
        /// Project id or path to remove
        id_or_path: String,
        /// Print the removed project entry as JSON
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub agent_home: String,
    pub added_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRegistry {
    #[serde(default = "default_registry_version")]
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<ProjectEntry>,
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        Self {
            version: default_registry_version(),
            projects: Vec::new(),
        }
    }
}

fn default_registry_version() -> u32 {
    1
}

pub async fn execute(_config: Arc<Config>, action: ProjectsAction) -> Result<()> {
    let registry_path = default_registry_path();
    match action {
        ProjectsAction::Add { path, name, json } => {
            let now = now_rfc3339();
            let entry = add_project_at(&registry_path, &path, name.as_deref(), &now)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
            } else {
                println!("Registered project:");
                print_entry_details(&entry);
                println!("Registry: {}", registry_path.display());
            }
        }
        ProjectsAction::List { json } => {
            let registry = load_registry(&registry_path)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&registry)?);
            } else {
                print_project_table(&registry.projects);
            }
        }
        ProjectsAction::Remove { id_or_path, json } => {
            let removed = remove_project(&registry_path, &id_or_path)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&removed)?);
            } else {
                println!("Removed project:");
                print_entry_details(&removed);
                println!("Registry: {}", registry_path.display());
            }
        }
    }
    Ok(())
}

fn default_registry_path() -> PathBuf {
    agent007_core::paths::agent007_global_home().join("projects.json")
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn add_project_at(
    registry_path: &Path,
    project_path: &Path,
    name: Option<&str>,
    now: &str,
) -> Result<ProjectEntry> {
    let canonical = canonical_project_dir(project_path)?;
    let path = canonical.to_string_lossy().to_string();
    let id = stable_project_id(&canonical);
    let display_name = clean_name(name).unwrap_or_else(|| derive_project_name(&canonical));
    let agent_home = canonical.join(".agent007").to_string_lossy().to_string();

    let mut registry = load_registry(registry_path)?;
    if let Some(existing) = registry
        .projects
        .iter_mut()
        .find(|entry| entry.path == path)
    {
        existing.name = display_name;
        existing.agent_home = agent_home;
        existing.last_seen_at = now.to_string();
        let updated = existing.clone();
        save_registry(registry_path, &registry)?;
        return Ok(updated);
    }

    let entry = ProjectEntry {
        id,
        name: display_name,
        path,
        agent_home,
        added_at: now.to_string(),
        last_seen_at: now.to_string(),
    };
    registry.projects.push(entry.clone());
    save_registry(registry_path, &registry)?;
    Ok(entry)
}

fn remove_project(registry_path: &Path, id_or_path: &str) -> Result<ProjectEntry> {
    let mut registry = load_registry(registry_path)?;
    let path_match = normalize_remove_path(id_or_path);
    let before = registry.projects.len();
    let Some(index) = registry.projects.iter().position(|entry| {
        entry.id == id_or_path
            || entry.path == id_or_path
            || path_match.as_deref() == Some(entry.path.as_str())
    }) else {
        anyhow::bail!(
            "project '{}' not found in {}",
            id_or_path,
            registry_path.display()
        );
    };
    let removed = registry.projects.remove(index);
    debug_assert_eq!(registry.projects.len() + 1, before);
    save_registry(registry_path, &registry)?;
    Ok(removed)
}

fn load_registry(registry_path: &Path) -> Result<ProjectRegistry> {
    if !registry_path.exists() {
        return Ok(ProjectRegistry::default());
    }
    let content = fs::read_to_string(registry_path)
        .with_context(|| format!("failed to read {}", registry_path.display()))?;
    if content.trim().is_empty() {
        return Ok(ProjectRegistry::default());
    }
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", registry_path.display()))
}

fn save_registry(registry_path: &Path, registry: &ProjectRegistry) -> Result<()> {
    if let Some(parent) = registry_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(registry)?;
    atomic_write(registry_path, &format!("{}\n", content))
        .with_context(|| format!("failed to write {}", registry_path.display()))
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{} has no file name", path.display()))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let tmp_path = parent.join(format!(
        ".{}.{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id(),
        nanos
    ));

    let write_result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .with_context(|| format!("failed to create {}", tmp_path.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("failed to write {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", tmp_path.display()))?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }

    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("failed to replace {}", path.display()))?;
    }

    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err).with_context(|| {
            format!(
                "failed to rename {} to {}",
                tmp_path.display(),
                path.display()
            )
        });
    }
    Ok(())
}

fn canonical_project_dir(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to resolve project path {}", path.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!("project path {} is not a directory", canonical.display());
    }
    Ok(canonical)
}

fn normalize_remove_path(value: &str) -> Option<String> {
    let path = Path::new(value);
    if let Ok(canonical) = path.canonicalize() {
        return Some(
            normalize_path_lexically(&canonical)
                .to_string_lossy()
                .to_string(),
        );
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    Some(
        normalize_path_lexically(&absolute)
            .to_string_lossy()
            .to_string(),
    )
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() && !path.is_absolute() {
                    normalized.push("..");
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

fn clean_name(name: Option<&str>) -> Option<String> {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn derive_project_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("project")
        .to_string()
}

fn stable_project_id(path: &Path) -> String {
    // FNV-1a over the canonical path string. This keeps IDs deterministic without
    // adding a new hashing dependency and is stable across process runs.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("proj-{hash:016x}")
}

fn print_entry_details(entry: &ProjectEntry) {
    println!("  id:         {}", entry.id);
    println!("  name:       {}", entry.name);
    println!("  path:       {}", entry.path);
    println!("  agent_home: {}", entry.agent_home);
}

fn print_project_table(projects: &[ProjectEntry]) {
    if projects.is_empty() {
        println!("No registered projects. Add one with `agent007 projects add <path>`.");
        return;
    }

    println!("{:<23} {:<24} {:<20} PATH", "ID", "NAME", "LAST SEEN");
    println!("{}", "-".repeat(96));
    for project in projects {
        println!(
            "{:<23} {:<24} {:<20} {}",
            project.id, project.name, project.last_seen_at, project.path
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_path(temp: &tempfile::TempDir) -> PathBuf {
        temp.path()
            .join("home")
            .join(".agent007")
            .join("projects.json")
    }

    #[test]
    fn add_project_creates_registry_entry() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("demo-project");
        fs::create_dir_all(&project).unwrap();
        let registry = registry_path(&temp);

        let entry = add_project_at(&registry, &project, None, "2026-06-17T10:00:00Z").unwrap();
        let canonical_project = project.canonicalize().unwrap();

        assert_eq!(entry.name, "demo-project");
        assert_eq!(
            entry.agent_home,
            canonical_project.join(".agent007").to_string_lossy()
        );
        assert_eq!(entry.added_at, "2026-06-17T10:00:00Z");
        assert_eq!(entry.last_seen_at, "2026-06-17T10:00:00Z");
        assert!(entry.id.starts_with("proj-"));

        let saved = load_registry(&registry).unwrap();
        assert_eq!(saved.projects, vec![entry]);
    }

    #[test]
    fn add_project_is_idempotent_and_updates_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("demo-project");
        fs::create_dir_all(&project).unwrap();
        let registry = registry_path(&temp);

        let first =
            add_project_at(&registry, &project, Some("Demo"), "2026-06-17T10:00:00Z").unwrap();
        let second = add_project_at(
            &registry,
            &project,
            Some("Renamed Demo"),
            "2026-06-17T10:01:00Z",
        )
        .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.name, "Renamed Demo");
        assert_eq!(second.added_at, "2026-06-17T10:00:00Z");
        assert_eq!(second.last_seen_at, "2026-06-17T10:01:00Z");

        let saved = load_registry(&registry).unwrap();
        assert_eq!(saved.projects.len(), 1);
        assert_eq!(saved.projects[0], second);
    }

    #[test]
    fn remove_project_by_id_and_path() {
        let temp = tempfile::tempdir().unwrap();
        let alpha = temp.path().join("alpha");
        let beta = temp.path().join("beta");
        fs::create_dir_all(&alpha).unwrap();
        fs::create_dir_all(&beta).unwrap();
        let registry = registry_path(&temp);

        let alpha_entry = add_project_at(&registry, &alpha, None, "2026-06-17T10:00:00Z").unwrap();
        let beta_entry = add_project_at(&registry, &beta, None, "2026-06-17T10:00:00Z").unwrap();

        let removed_alpha = remove_project(&registry, &alpha_entry.id).unwrap();
        assert_eq!(removed_alpha, alpha_entry);

        let removed_beta = remove_project(&registry, beta.to_str().unwrap()).unwrap();
        assert_eq!(removed_beta, beta_entry);

        let saved = load_registry(&registry).unwrap();
        assert!(saved.projects.is_empty());
    }

    #[test]
    fn remove_project_by_path_works_after_directory_is_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("demo-project");
        fs::create_dir_all(&project).unwrap();
        let registry = registry_path(&temp);

        let entry = add_project_at(&registry, &project, None, "2026-06-17T10:00:00Z").unwrap();
        fs::remove_dir_all(&project).unwrap();

        let equivalent_missing_path = Path::new(&entry.path).join("nested").join("..");
        let removed = remove_project(&registry, equivalent_missing_path.to_str().unwrap()).unwrap();

        assert_eq!(removed, entry);
        assert!(load_registry(&registry).unwrap().projects.is_empty());
    }

    #[test]
    fn save_registry_uses_temp_file_without_leaving_tmp_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_path(&temp);
        let project = temp.path().join("demo-project");
        fs::create_dir_all(&project).unwrap();

        let entry = add_project_at(&registry, &project, None, "2026-06-17T10:00:00Z").unwrap();

        let saved = load_registry(&registry).unwrap();
        assert_eq!(saved.projects, vec![entry]);
        let parent = registry.parent().unwrap();
        let leftovers: Vec<_> = fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    }

    #[test]
    fn remove_project_reports_missing_target() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_path(&temp);

        let err = remove_project(&registry, "proj-missing").unwrap_err();

        assert!(err.to_string().contains("proj-missing"));
    }

    #[test]
    fn stable_project_id_is_deterministic() {
        let path = Path::new("/tmp/agent007-demo");

        assert_eq!(stable_project_id(path), stable_project_id(path));
        assert_ne!(
            stable_project_id(path),
            stable_project_id(Path::new("/tmp/other"))
        );
    }
}
