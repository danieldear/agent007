use crate::{AgentDef, CustomAgentError};
use std::path::Path;

pub fn load_agent_def(path: &Path) -> Result<AgentDef, CustomAgentError> {
    let content = std::fs::read_to_string(path)?;
    toml::from_str::<AgentDef>(&content).map_err(|e| CustomAgentError::ParseError {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })
}

pub fn load_all(agents_dir: &Path) -> Result<Vec<AgentDef>, CustomAgentError> {
    let mut defs = Vec::new();
    if !agents_dir.exists() {
        return Ok(defs);
    }
    for entry in std::fs::read_dir(agents_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            defs.push(load_agent_def(&path)?);
        }
    }
    Ok(defs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_agent_toml(dir: &std::path::Path, filename: &str, content: &str) {
        fs::write(dir.join(filename), content).unwrap();
    }

    const VALID_TOML: &str = r#"
        name = "TestAgent"
        type = "worker"
        system_prompt = "Test."
    "#;

    const INVALID_TOML: &str = r#"
        type = "worker"
        system_prompt = "Missing name field."
    "#;

    #[test]
    fn load_agent_def_valid() {
        let dir = tempdir().unwrap();
        write_agent_toml(dir.path(), "test.toml", VALID_TOML);
        let def = load_agent_def(&dir.path().join("test.toml")).unwrap();
        assert_eq!(def.name, "TestAgent");
    }

    #[test]
    fn load_agent_def_parse_error() {
        let dir = tempdir().unwrap();
        write_agent_toml(dir.path(), "bad.toml", INVALID_TOML);
        let err = load_agent_def(&dir.path().join("bad.toml")).unwrap_err();
        assert!(matches!(err, crate::CustomAgentError::ParseError { .. }));
    }

    #[test]
    fn load_agent_def_missing_file() {
        let dir = tempdir().unwrap();
        let err = load_agent_def(&dir.path().join("nonexistent.toml")).unwrap_err();
        assert!(matches!(err, crate::CustomAgentError::Io(_)));
    }

    #[test]
    fn load_all_returns_only_toml_files() {
        let dir = tempdir().unwrap();
        write_agent_toml(dir.path(), "agent_a.toml", VALID_TOML);
        fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let defs = load_all(dir.path()).unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "TestAgent");
    }

    #[test]
    fn load_all_empty_dir() {
        let dir = tempdir().unwrap();
        let defs = load_all(dir.path()).unwrap();
        assert!(defs.is_empty());
    }

    #[test]
    fn load_all_skips_invalid_files_with_error() {
        // load_all should return Err if any file fails to parse
        let dir = tempdir().unwrap();
        write_agent_toml(dir.path(), "good.toml", VALID_TOML);
        write_agent_toml(dir.path(), "bad.toml", INVALID_TOML);
        let result = load_all(dir.path());
        assert!(result.is_err());
    }
}
