// crates/personas/src/loader.rs
use crate::error::PersonaError;
use agent007_core::PersonaSpec;
use std::path::Path;

/// Deserialisation target — mirrors PersonaSpec for TOML parsing.
#[derive(serde::Deserialize)]
struct PersonaFile {
    name: String,
    description: String,
    system_prompt: String,
    preferred_model: String,
    #[serde(default)]
    allowed_tools: Vec<String>,
}

/// Load all PersonaSpec overrides from *.toml files in user_dir.
/// Files that fail to parse return PersonaError::ParseError.
/// Non-.toml files are silently ignored.
pub fn load_user_overrides(user_dir: &Path) -> Result<Vec<PersonaSpec>, PersonaError> {
    let mut specs = Vec::new();

    let entries = std::fs::read_dir(user_dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }

        let content = std::fs::read_to_string(&path)?;
        let pf: PersonaFile = toml::from_str(&content).map_err(|e| PersonaError::ParseError {
            path: path.clone(),
            reason: e.to_string(),
        })?;

        specs.push(PersonaSpec {
            name: pf.name,
            description: pf.description,
            system_prompt: pf.system_prompt,
            preferred_model: pf.preferred_model,
            allowed_tools: pf.allowed_tools,
        });
    }

    Ok(specs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn loads_valid_toml_persona_file() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("my-coder.toml"),
            r#"
name = "MyCoder"
description = "Custom Rust specialist"
system_prompt = "You are an expert in Rust embedded systems."
preferred_model = "codex"
allowed_tools = ["bash", "file_read", "file_write"]
"#,
        )
        .unwrap();

        let specs = load_user_overrides(dir.path()).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "MyCoder");
        assert_eq!(specs[0].preferred_model, "codex");
        assert_eq!(
            specs[0].allowed_tools,
            vec!["bash", "file_read", "file_write"]
        );
    }

    #[test]
    fn ignores_non_toml_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("notes.txt"), "not a toml persona").unwrap();
        fs::write(
            dir.path().join("valid.toml"),
            r#"
name = "Valid"
description = "valid persona"
system_prompt = "You are valid."
preferred_model = "claude"
allowed_tools = []
"#,
        )
        .unwrap();

        let specs = load_user_overrides(dir.path()).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "Valid");
    }

    #[test]
    fn returns_parse_error_for_invalid_toml() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bad.toml"), "this is not valid toml ][").unwrap();

        let result = load_user_overrides(dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("bad.toml"),
            "error should mention the file path"
        );
    }

    #[test]
    fn empty_directory_returns_empty_vec() {
        let dir = TempDir::new().unwrap();
        let specs = load_user_overrides(dir.path()).unwrap();
        assert!(specs.is_empty());
    }

    #[test]
    fn loads_multiple_toml_files() {
        let dir = TempDir::new().unwrap();
        for i in 0..3u8 {
            fs::write(
                dir.path().join(format!("persona{}.toml", i)),
                format!(
                    r#"name = "Persona{i}"
description = "desc {i}"
system_prompt = "prompt {i}"
preferred_model = "claude"
allowed_tools = []
"#,
                ),
            )
            .unwrap();
        }
        let specs = load_user_overrides(dir.path()).unwrap();
        assert_eq!(specs.len(), 3);
    }
}
