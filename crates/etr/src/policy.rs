use serde_json::Value;
use std::path::{Component, Path, PathBuf};

pub struct PolicyEngine {
    pub workspace_root: PathBuf,
}
#[derive(Debug)]
pub enum PolicyResult {
    Allowed,
    Denied(String),
}

/// The same path vocabulary is used for normalization and validation, including
/// nested objects/arrays. Values that are inline data are not treated as paths.
pub fn path_key(k: &str) -> bool {
    if k == "jq_path" {
        return false;
    } // JSON selector, not a filesystem path.
    matches!(
        k,
        "path" | "file" | "dir" | "directory" | "root" | "paths" | "files"
    ) || k.starts_with("path_")
        || k.ends_with("_path")
        || k.ends_with("_file")
        || k.starts_with("file_")
}
impl PolicyEngine {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
    pub fn resolve(&self, raw: &str) -> Result<PathBuf, String> {
        let root = self
            .workspace_root
            .canonicalize()
            .map_err(|_| "Workspace is unavailable".to_string())?;
        let p = Path::new(raw);
        if raw.is_empty()
            || raw.len() > 16384
            || raw.contains('\0')
            || p.components().any(|c| matches!(c, Component::ParentDir))
        {
            return Err("Invalid or traversing path".into());
        }
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        };
        let mut ancestor = p.as_path();
        loop {
            match std::fs::symlink_metadata(ancestor) {
                Ok(_) => {
                    let canonical = ancestor
                        .canonicalize()
                        .map_err(|_| "Unresolvable path or symlink".to_string())?;
                    if !canonical.starts_with(&root) {
                        return Err("Path is outside the workspace".into());
                    }
                    // Keep the canonical existing ancestor, not a lexical prefix.
                    let suffix = p.strip_prefix(ancestor).map_err(|_| "Invalid path")?;
                    return Ok(if suffix.as_os_str().is_empty() {
                        canonical
                    } else {
                        canonical.join(suffix)
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    ancestor = ancestor.parent().ok_or("No existing path ancestor")?;
                }
                Err(_) => return Err("Cannot validate path".into()),
            }
        }
    }
    pub fn check_path(&self, raw: &str) -> PolicyResult {
        match self.resolve(raw) {
            Ok(_) => PolicyResult::Allowed,
            Err(e) => PolicyResult::Denied(e),
        }
    }
    pub fn prepare(&self, input: &Value) -> Result<Value, String> {
        struct Budget {
            nodes: usize,
            bytes: usize,
        }
        impl Budget {
            fn charge(&mut self, n: usize) -> Result<(), String> {
                self.bytes = self
                    .bytes
                    .checked_sub(n)
                    .ok_or("Tool input exceeds byte budget")?;
                Ok(())
            }
        }
        fn visit(
            policy: &PolicyEngine,
            v: &Value,
            pathy: bool,
            data: bool,
            depth: usize,
            budget: &mut Budget,
        ) -> Result<Value, String> {
            budget.nodes = budget
                .nodes
                .checked_sub(1)
                .ok_or("Tool input exceeds node budget")?;
            if depth > 32 {
                return Err("Tool input exceeds nesting budget".into());
            }
            match v {
                Value::String(s) => {
                    if pathy {
                        let path = policy.resolve(s)?.to_string_lossy().into_owned();
                        budget.charge(path.len())?;
                        Ok(Value::String(path))
                    } else {
                        budget.charge(s.len())?;
                        Ok(v.clone())
                    }
                }
                Value::Array(a) => {
                    if a.len() > crate::budget::ENTRIES {
                        return Err("Tool input exceeds item budget".into());
                    }
                    a.iter()
                        .map(|v| visit(policy, v, pathy, data, depth + 1, budget))
                        .collect::<Result<Vec<_>, _>>()
                        .map(Value::Array)
                }
                Value::Object(o) => {
                    if o.len() > crate::budget::ENTRIES {
                        return Err("Tool input exceeds item budget".into());
                    }
                    o.iter()
                        .map(|(k, v)| {
                            budget.charge(k.len())?;
                            let inline = data
                                || (!pathy
                                    && matches!(
                                        k.as_str(),
                                        "baseline"
                                            | "candidate"
                                            | "rows"
                                            | "left_rows"
                                            | "right_rows"
                                    ));
                            Ok((
                                k.clone(),
                                visit(
                                    policy,
                                    v,
                                    !inline && (pathy || path_key(k)),
                                    inline,
                                    depth + 1,
                                    budget,
                                )?,
                            ))
                        })
                        .collect::<Result<serde_json::Map<_, _>, String>>()
                        .map(Value::Object)
                }
                _ => Ok(v.clone()),
            }
        }
        visit(
            self,
            input,
            false,
            false,
            0,
            &mut Budget {
                nodes: 100_000,
                bytes: 1024 * 1024,
            },
        )
    }
    pub fn check(&self, _tool: &str, input: &Value) -> PolicyResult {
        match self.prepare(input) {
            Ok(_) => PolicyResult::Allowed,
            Err(e) => PolicyResult::Denied(e),
        }
    }
}

#[cfg(test)]
mod confinement_regressions {
    use super::*;
    #[test]
    fn outside_absolute_nested_and_nonexistent_paths_are_denied() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let outside = d.path().join("outside");
        std::fs::write(&outside, "private fixture").unwrap();
        let policy = PolicyEngine::new(root);
        for input in [
            serde_json::json!({"path":outside}),
            serde_json::json!({"files":[outside]}),
            serde_json::json!({"paths":{"a":outside}}),
            serde_json::json!({"nested":{"file":outside}}),
            serde_json::json!({"path":d.path().join("missing/file")}),
        ] {
            assert!(
                matches!(policy.check("etr.grep", &input), PolicyResult::Denied(_)),
                "{input}"
            );
        }
    }
    #[test]
    fn inline_data_keeps_its_path_fields_but_obeys_all_budgets() {
        let d = tempfile::tempdir().unwrap();
        let p = PolicyEngine::new(d.path().to_path_buf());
        let good = serde_json::json!({"rows":[{"path":"/outside-is-only-data"}]});
        assert_eq!(p.prepare(&good).unwrap(), good);
        let mut deep = serde_json::json!({"path":"/data"});
        for _ in 0..34 {
            deep = serde_json::json!({"nested":deep});
        }
        assert!(p.prepare(&serde_json::json!({"baseline":deep})).is_err());
        assert!(p
            .prepare(
                &serde_json::json!({"rows":vec![serde_json::json!(1);crate::budget::ENTRIES+1]})
            )
            .is_err());
        assert!(p
            .prepare(&serde_json::json!({"candidate":{"text":"x".repeat(1024*1024+1)}}))
            .is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_denied_without_dotdot() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let outside = d.path().join("outside");
        std::fs::write(&outside, "private fixture").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert!(matches!(
            PolicyEngine::new(root).check_path("link"),
            PolicyResult::Denied(_)
        ));
    }
}
