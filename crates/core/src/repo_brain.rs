use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::repo_filter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoBrain {
    pub project_name: String,
    pub root: String,
    pub ecosystems: Vec<String>,
    pub entrypoints: Vec<String>,
    pub workflows: Vec<String>,
    pub skills: Vec<String>,
    pub memory_notes: Vec<String>,
    pub conventions: Vec<String>,
    pub recommended_commands: Vec<String>,
    pub summary: String,
}

pub struct RepoBrainBuilder {
    root: PathBuf,
    agent_home: PathBuf,
}

impl RepoBrainBuilder {
    pub fn new(root: impl Into<PathBuf>, agent_home: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            agent_home: agent_home.into(),
        }
    }

    pub fn build(&self) -> Result<RepoBrain, CoreError> {
        let project_name = self
            .root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("project")
            .to_string();
        let ecosystems = self.detect_ecosystems();
        let entrypoints = self.find_entrypoints();
        let workflows =
            collect_catalog_stems(&self.agent_home, "workflows", &["toml", "yaml", "yml"])?;
        let skills = collect_catalog_stems(&self.agent_home, "skills", &["md"])?;
        let mut memory_notes =
            collect_file_stems(&self.agent_home.join("memory").join("project"), &["md"])?;
        memory_notes.extend(collect_sqlite_memory_keys(
            &self.agent_home.join("memory").join("memory.db"),
            "project",
        )?);
        memory_notes.sort();
        memory_notes.dedup();
        let memory_note_count = memory_notes.len();
        let memory_key_limit = repo_filter::repo_brain_memory_key_limit();
        if memory_notes.len() > memory_key_limit {
            let omitted = memory_notes.len() - memory_key_limit;
            memory_notes.truncate(memory_key_limit);
            memory_notes.push(format!("... {omitted} more memory key(s) omitted"));
        }
        let conventions = self.collect_conventions();
        let recommended_commands = recommended_commands(&ecosystems);
        let ecosystem_label = if ecosystems.is_empty() {
            "mixed".to_string()
        } else {
            ecosystems.join("/")
        };
        let summary = format!(
            "{} is a {} project with {} workflow(s), {} skill(s), and {} project memory note(s).",
            project_name,
            ecosystem_label,
            workflows.len(),
            skills.len(),
            memory_note_count,
        );

        Ok(RepoBrain {
            project_name,
            root: self.root.display().to_string(),
            ecosystems,
            entrypoints,
            workflows,
            skills,
            memory_notes,
            conventions,
            recommended_commands,
            summary,
        })
    }

    fn detect_ecosystems(&self) -> Vec<String> {
        let mut ecosystems = Vec::new();
        if self.root.join("Cargo.toml").exists() {
            ecosystems.push("rust".to_string());
        }
        if self.root.join("package.json").exists() {
            ecosystems.push("node".to_string());
        }
        if self.root.join("pyproject.toml").exists() || self.root.join("requirements.txt").exists()
        {
            ecosystems.push("python".to_string());
        }
        if self.root.join("go.mod").exists() {
            ecosystems.push("go".to_string());
        }
        ecosystems
    }

    fn find_entrypoints(&self) -> Vec<String> {
        let candidates = [
            "AGENTS.md",
            "README.md",
            "Cargo.toml",
            "package.json",
            "src/main.rs",
            "src/lib.rs",
            "src/main.ts",
            "src/index.ts",
            "main.py",
            "go.mod",
        ];
        let mut hits = Vec::new();
        for candidate in candidates {
            if self.root.join(candidate).exists() {
                hits.push(candidate.to_string());
            }
        }
        if self.root.join("crates").is_dir() {
            hits.push("crates/".to_string());
        }
        if self.root.join("tests").is_dir() {
            hits.push("tests/".to_string());
        }
        hits
    }

    fn collect_conventions(&self) -> Vec<String> {
        let mut notes = Vec::new();
        for path in [self.root.join("AGENTS.md"), self.root.join("README.md")] {
            if let Ok(raw) = fs::read_to_string(&path) {
                for line in raw.lines().map(str::trim) {
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    notes.push(line.to_string());
                    if notes.len() >= 8 {
                        return notes;
                    }
                }
            }
        }
        notes
    }
}

fn collect_file_stems(dir: &Path, exts: &[&str]) -> Result<Vec<String>, CoreError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    let entries = fs::read_dir(dir).map_err(|error| CoreError::io(dir, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| CoreError::io(dir, error))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !exts
            .iter()
            .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

fn collect_catalog_stems(home: &Path, kind: &str, exts: &[&str]) -> Result<Vec<String>, CoreError> {
    let mut names = collect_file_stems(&home.join(kind), exts)?;
    for dir in crate::paths::enabled_pack_asset_dirs(home, kind) {
        names.extend(collect_file_stems(&dir, exts)?);
    }
    names.sort();
    names.dedup();
    Ok(names)
}

fn collect_sqlite_memory_keys(db_path: &Path, namespace: &str) -> Result<Vec<String>, CoreError> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|error| CoreError::io(db_path, std::io::Error::other(error)))?;
    let mut stmt = match conn.prepare("SELECT key FROM memory WHERE namespace = ?1") {
        Ok(stmt) => stmt,
        Err(_) => return Ok(Vec::new()),
    };
    let rows = stmt
        .query_map([namespace], |row| row.get::<_, String>(0))
        .map_err(|error| CoreError::io(db_path, std::io::Error::other(error)))?;
    let mut keys = Vec::new();
    for row in rows.flatten() {
        keys.push(row);
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
}

fn recommended_commands(ecosystems: &[String]) -> Vec<String> {
    let mut commands = Vec::new();
    if ecosystems.iter().any(|item| item == "rust") {
        commands.push("cargo build".to_string());
        commands.push("cargo test".to_string());
    }
    if ecosystems.iter().any(|item| item == "node") {
        commands.push("npm test".to_string());
        commands.push("npm run build".to_string());
    }
    if ecosystems.iter().any(|item| item == "python") {
        commands.push("pytest".to_string());
    }
    if ecosystems.iter().any(|item| item == "go") {
        commands.push("go test ./...".to_string());
    }
    if commands.is_empty() {
        commands.push("git status".to_string());
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_repo_brain_from_project_layout() {
        let root = tempfile::tempdir().unwrap();
        let agent_home = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(
            root.path().join("AGENTS.md"),
            "Use cargo test\nPrefer concise diffs\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join("src")).unwrap();
        fs::write(root.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(agent_home.path().join("workflows")).unwrap();
        fs::write(
            agent_home.path().join("workflows").join("ship.toml"),
            "name='ship'\n",
        )
        .unwrap();
        fs::create_dir_all(agent_home.path().join("skills")).unwrap();
        fs::write(
            agent_home.path().join("skills").join("review.md"),
            "---\n---\n",
        )
        .unwrap();

        let brain = RepoBrainBuilder::new(root.path(), agent_home.path())
            .build()
            .unwrap();
        assert!(brain.ecosystems.contains(&"rust".to_string()));
        assert!(brain.entrypoints.contains(&"src/main.rs".to_string()));
        assert!(brain.workflows.contains(&"ship".to_string()));
        assert!(brain.skills.contains(&"review".to_string()));
    }

    #[test]
    fn repo_brain_counts_sqlite_project_memory_keys() {
        let root = tempfile::tempdir().unwrap();
        let agent_home = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        let memory_dir = agent_home.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        let db_path = memory_dir.join("memory.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE memory (
                namespace TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                access_count INTEGER NOT NULL DEFAULT 0,
                entry_type TEXT NOT NULL DEFAULT 'semantic',
                summary TEXT NOT NULL DEFAULT '',
                expires_after TEXT,
                confidence REAL NOT NULL DEFAULT 1.0,
                words TEXT NOT NULL DEFAULT '[]',
                related_to TEXT NOT NULL DEFAULT '[]',
                PRIMARY KEY (namespace, key)
            );
            INSERT INTO memory (namespace, key, value, created_at, updated_at)
            VALUES ('project', 'database_pool', 'Use bounded DB pools', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
            ",
        )
        .unwrap();

        let brain = RepoBrainBuilder::new(root.path(), agent_home.path())
            .build()
            .unwrap();

        assert!(brain.memory_notes.contains(&"database_pool".to_string()));
        assert!(brain.summary.contains("1 project memory note"));
    }

    #[test]
    fn repo_brain_caps_memory_key_inventory_metadata() {
        std::env::remove_var("AGENT007_REPO_BRAIN_MAX_MEMORY_KEYS");
        let root = tempfile::tempdir().unwrap();
        let agent_home = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        let project_memory = agent_home.path().join("memory").join("project");
        fs::create_dir_all(&project_memory).unwrap();
        for idx in 0..70 {
            fs::write(project_memory.join(format!("note_{idx:02}.md")), "memory\n").unwrap();
        }

        let brain = RepoBrainBuilder::new(root.path(), agent_home.path())
            .build()
            .unwrap();

        assert_eq!(brain.memory_notes.len(), 65);
        assert!(brain
            .memory_notes
            .last()
            .is_some_and(|note| note.contains("6 more memory key(s) omitted")));
        assert!(brain.summary.contains("70 project memory note"));
    }
}
