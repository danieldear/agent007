use serde_json::Value;
use std::path::Path;

pub struct PolicyEngine {
    pub workspace_root: std::path::PathBuf,
}

pub enum PolicyResult {
    Allowed,
    Denied(String),
}

impl PolicyEngine {
    pub fn new(workspace_root: std::path::PathBuf) -> Self {
        Self { workspace_root }
    }

    pub fn check_path(&self, raw: &str) -> PolicyResult {
        let p = if Path::new(raw).is_absolute() {
            Path::new(raw).to_path_buf()
        } else {
            self.workspace_root.join(raw)
        };
        match p.canonicalize() {
            Ok(canonical) => {
                if raw.contains("..") {
                    let ws = self
                        .workspace_root
                        .canonicalize()
                        .unwrap_or_else(|_| self.workspace_root.clone());
                    if !canonical.starts_with(&ws) {
                        return PolicyResult::Denied(format!("Path traversal rejected: {raw}"));
                    }
                }
                PolicyResult::Allowed
            }
            Err(_) => PolicyResult::Allowed, // file doesn't exist yet — allow (creation case)
        }
    }

    pub fn check(&self, _tool: &str, input: &Value) -> PolicyResult {
        if let Some(obj) = input.as_object() {
            for (k, v) in obj {
                if k == "path"
                    || k.ends_with("_path")
                    || k == "path_a"
                    || k == "path_b"
                    || k == "root"
                {
                    if let Some(s) = v.as_str() {
                        if let PolicyResult::Denied(reason) = self.check_path(s) {
                            return PolicyResult::Denied(reason);
                        }
                    }
                }
            }
        }
        PolicyResult::Allowed
    }
}
