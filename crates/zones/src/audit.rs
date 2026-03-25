// crates/zones/src/audit.rs
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;

use crate::error::ZonesError;

#[derive(Debug, Serialize)]
pub struct AuditEntry {
    pub ts: String,
    pub agent: String,
    pub action: String,   // "read" | "write" | "delete"
    pub path: String,
    pub zone: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<bool>,
}

impl AuditEntry {
    /// Convenience constructor with current timestamp.
    pub fn now(
        agent: impl Into<String>,
        action: impl Into<String>,
        path: impl Into<String>,
        zone: impl Into<String>,
        allowed: bool,
    ) -> Self {
        Self {
            ts: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            agent: agent.into(),
            action: action.into(),
            path: path.into(),
            zone: zone.into(),
            allowed,
            blocked: if allowed { None } else { Some(true) },
        }
    }
}

pub struct AuditLogger {
    path: PathBuf,
}

impl AuditLogger {
    pub fn new(log_path: &Path) -> Self {
        Self { path: log_path.to_path_buf() }
    }

    /// Append one AuditEntry as a newline-delimited JSON record.
    pub fn log(&self, entry: &AuditEntry) -> Result<(), ZonesError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Read all raw lines from the audit log.
    /// Returns an empty vec if the file does not exist.
    pub fn read_lines(&self) -> Result<Vec<String>, ZonesError> {
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let content = std::fs::read_to_string(&self.path)?;
        Ok(content
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_logger(dir: &TempDir) -> AuditLogger {
        AuditLogger::new(&dir.path().join("audit").join("audit.log"))
    }

    #[test]
    fn log_creates_file_and_appends_json() {
        let dir = TempDir::new().unwrap();
        let logger = make_logger(&dir);

        let entry = AuditEntry {
            ts:      "2026-03-16T10:00:00Z".to_string(),
            agent:   "WorkerAgent".to_string(),
            action:  "read".to_string(),
            path:    "src/auth/login.rs".to_string(),
            zone:    "readonly".to_string(),
            allowed: true,
            blocked: None,
        };
        logger.log(&entry).unwrap();

        let lines = logger.read_lines().unwrap();
        assert_eq!(lines.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(parsed["agent"], "WorkerAgent");
        assert_eq!(parsed["action"], "read");
        assert_eq!(parsed["zone"],   "readonly");
        assert_eq!(parsed["allowed"], true);
        assert!(parsed.get("blocked").is_none(), "blocked should be omitted when allowed=true");
    }

    #[test]
    fn log_appends_multiple_entries() {
        let dir = TempDir::new().unwrap();
        let logger = make_logger(&dir);

        for i in 0..5 {
            let entry = AuditEntry::now(
                "Agent",
                "write",
                format!("src/file{}.rs", i),
                "unrestricted",
                true,
            );
            logger.log(&entry).unwrap();
        }

        let lines = logger.read_lines().unwrap();
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn log_blocked_entry_has_blocked_true() {
        let dir = TempDir::new().unwrap();
        let logger = make_logger(&dir);

        let entry = AuditEntry::now(
            "WorkerAgent",
            "write",
            "src/auth/login.rs",
            "readonly",
            false,
        );
        logger.log(&entry).unwrap();

        let lines = logger.read_lines().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(parsed["allowed"], false);
        assert_eq!(parsed["blocked"], true);
    }

    #[test]
    fn read_lines_returns_empty_when_no_file() {
        let dir = TempDir::new().unwrap();
        let logger = AuditLogger::new(&dir.path().join("nonexistent").join("audit.log"));
        let lines = logger.read_lines().unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn log_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let deep_path = dir.path().join("a").join("b").join("c").join("audit.log");
        let logger = AuditLogger::new(&deep_path);
        let entry = AuditEntry::now("A", "read", "f.rs", "unrestricted", true);
        // Should not fail even though directories don't exist
        logger.log(&entry).unwrap();
        assert!(deep_path.exists());
    }

    #[test]
    fn audit_entry_now_sets_ts_and_blocked() {
        let allowed_entry = AuditEntry::now("A", "read", "f.rs", "unrestricted", true);
        assert!(allowed_entry.blocked.is_none());

        let blocked_entry = AuditEntry::now("A", "write", "secrets/x", "forbidden", false);
        assert_eq!(blocked_entry.blocked, Some(true));
    }
}
