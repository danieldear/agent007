use crate::error::WorkflowError;
use crate::types::WorkflowDef;
use std::path::{Path, PathBuf};

pub struct WorkflowLoader {
    pub dir: PathBuf,
}

impl WorkflowLoader {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Load a WorkflowDef from a file path. Supports .toml, .yaml, and .yml.
    pub fn load_file(&self, path: &Path) -> Result<WorkflowDef, WorkflowError> {
        let raw = std::fs::read_to_string(path)?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let def = match ext {
            "yaml" | "yml" => {
                serde_yaml::from_str::<WorkflowDef>(&raw).map_err(|e| WorkflowError::ParseError {
                    path: path.to_path_buf(),
                    reason: e.to_string(),
                })
            }
            _ => toml::from_str::<WorkflowDef>(&raw).map_err(|e| WorkflowError::ParseError {
                path: path.to_path_buf(),
                reason: e.to_string(),
            }),
        }?;
        def.validate_schema()?;
        Ok(def)
    }

    /// Load a workflow by short name.
    /// Resolution order: <name>.yaml → <name>.yml → <name>.toml
    pub fn load_named(&self, name: &str) -> Result<WorkflowDef, WorkflowError> {
        for ext in &["yaml", "yml", "toml"] {
            let path = self.dir.join(format!("{}.{}", name, ext));
            if path.exists() {
                return self.load_file(&path);
            }
        }
        Err(WorkflowError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("workflow '{}' not found (.yaml/.yml/.toml)", name),
        )))
    }

    /// Resolve a workflow from ordered directories, returning the first match.
    pub fn load_named_from_dirs(
        dirs: impl IntoIterator<Item = impl Into<PathBuf>>,
        name: &str,
    ) -> Result<WorkflowDef, WorkflowError> {
        for dir in dirs.into_iter().map(Into::into) {
            match Self::new(dir).load_named(name) {
                Ok(def) => return Ok(def),
                Err(WorkflowError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Err(WorkflowError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("workflow '{name}' not found in configured catalogs"),
        )))
    }

    /// Return all short names (stem of .toml/.yaml/.yml files) in the loader directory.
    /// Returns empty vec if the directory does not exist.
    pub fn list_names(&self) -> Result<Vec<String>, WorkflowError> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }
        let mut names = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "toml" | "yaml" | "yml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if seen.insert(stem.to_string()) {
                        names.push(stem.to_string());
                    }
                }
            }
        }
        names.sort();
        Ok(names)
    }

    /// Load all workflows from the directory.
    pub fn load_all(&self) -> Result<Vec<WorkflowDef>, WorkflowError> {
        let names = self.list_names()?;
        names.iter().map(|n| self.load_named(n)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const SAMPLE_TOML: &str = r#"
name = "Sample"
[[steps]]
id = "s1"
agent = "Researcher"
prompt = "research {{task}}"
output = "notes"
"#;

    #[test]
    fn load_from_file_returns_workflow_def() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sample.toml");
        fs::write(&path, SAMPLE_TOML).unwrap();

        let loader = WorkflowLoader::new(dir.path().to_path_buf());
        let def = loader.load_file(&path).unwrap();
        assert_eq!(def.name, "Sample");
    }

    #[test]
    fn load_from_missing_file_returns_io_error() {
        let loader = WorkflowLoader::new(std::path::PathBuf::from("/tmp/nonexistent"));
        let err = loader
            .load_file(std::path::Path::new("/tmp/does_not_exist.toml"))
            .unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::Io(_)));
    }

    #[test]
    fn load_invalid_toml_returns_parse_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        fs::write(&path, "not valid toml {{ [[").unwrap();

        let loader = WorkflowLoader::new(dir.path().to_path_buf());
        let err = loader.load_file(&path).unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::ParseError { .. }
        ));
    }

    #[test]
    fn load_by_name_resolves_from_dir() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("my-flow.toml"), SAMPLE_TOML).unwrap();

        let loader = WorkflowLoader::new(dir.path().to_path_buf());
        let def = loader.load_named("my-flow").unwrap();
        assert_eq!(def.name, "Sample");
    }

    #[test]
    fn load_named_from_dirs_uses_first_match() {
        let manual = tempdir().unwrap();
        let pack = tempdir().unwrap();
        fs::write(
            manual.path().join("flow.toml"),
            SAMPLE_TOML.replace("Sample", "Manual"),
        )
        .unwrap();
        fs::write(
            pack.path().join("flow.toml"),
            SAMPLE_TOML.replace("Sample", "Pack"),
        )
        .unwrap();

        let def = WorkflowLoader::load_named_from_dirs(
            [manual.path().to_path_buf(), pack.path().to_path_buf()],
            "flow",
        )
        .unwrap();
        assert_eq!(def.name, "Manual");
    }

    #[test]
    fn list_workflows_returns_all_toml_names() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("alpha.toml"), SAMPLE_TOML).unwrap();
        fs::write(dir.path().join("beta.toml"), SAMPLE_TOML).unwrap();
        fs::write(dir.path().join("ignore.txt"), "").unwrap();

        let loader = WorkflowLoader::new(dir.path().to_path_buf());
        let mut names = loader.list_names().unwrap();
        names.sort();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn list_empty_dir_returns_empty_vec() {
        let dir = tempdir().unwrap();
        let loader = WorkflowLoader::new(dir.path().to_path_buf());
        let names = loader.list_names().unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn load_workflow_missing_prompt_and_skill_returns_schema_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        fs::write(
            &path,
            r#"name = "Bad"
[[steps]]
id = "s1"
agent = "Coder"
output = "result"
"#,
        )
        .unwrap();
        let loader = WorkflowLoader::new(dir.path().to_path_buf());
        let err = loader.load_file(&path).unwrap_err();
        assert!(
            matches!(err, crate::error::WorkflowError::SchemaError { .. }),
            "expected SchemaError, got {:?}",
            err
        );
    }

    #[test]
    fn load_sub_workflow_missing_workflow_field_returns_schema_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        fs::write(
            &path,
            r#"name = "Bad"
[[steps]]
id = "sub"
agent = "Coder"
type = "sub-workflow"
prompt = "do something"
output = "result"
"#,
        )
        .unwrap();
        let loader = WorkflowLoader::new(dir.path().to_path_buf());
        let err = loader.load_file(&path).unwrap_err();
        assert!(
            matches!(err, crate::error::WorkflowError::SchemaError { .. }),
            "expected SchemaError, got {:?}",
            err
        );
    }

    #[test]
    fn load_workflow_with_empty_name_returns_schema_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty_name.toml");
        fs::write(
            &path,
            r#"name = ""
[[steps]]
id = "s1"
agent = "Coder"
prompt = "do something"
output = "result"
"#,
        )
        .unwrap();
        let loader = WorkflowLoader::new(dir.path().to_path_buf());
        let err = loader.load_file(&path).unwrap_err();
        assert!(
            matches!(err, crate::error::WorkflowError::SchemaError { .. }),
            "expected SchemaError, got {:?}",
            err
        );
    }
}
