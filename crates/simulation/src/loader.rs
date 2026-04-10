use std::path::PathBuf;

use crate::error::SimulationError;
use crate::types::SimulationTemplate;

// Embedded built-in templates (compile-time)
const BUILTIN_WIFI_RTT: &str = include_str!("../templates/wifi-rtt.toml");
const BUILTIN_WIFI_ROAMING: &str = include_str!("../templates/wifi-roaming.toml");

/// Maps built-in template names to their embedded TOML content.
fn builtin_templates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("wifi-rtt", BUILTIN_WIFI_RTT),
        ("wifi-roaming", BUILTIN_WIFI_ROAMING),
    ]
}

/// Resolves simulation templates from built-in embedded strings or from the
/// filesystem (custom templates stored under `custom_dir`).
pub struct TemplateLoader {
    /// Optional override directory for built-in templates (useful in tests).
    pub builtin_dir: Option<PathBuf>,
    /// Directory for user-provided custom templates.
    /// Defaults to project-local or global `.agent007/simulations/custom/` when `None`.
    pub custom_dir: Option<PathBuf>,
}

impl TemplateLoader {
    /// Create a loader that only uses embedded built-in templates.
    pub fn new_builtin_only() -> Self {
        Self {
            builtin_dir: None,
            custom_dir: None,
        }
    }

    /// Create a loader with a custom template directory.
    pub fn with_custom_dir(custom_dir: PathBuf) -> Self {
        Self {
            builtin_dir: None,
            custom_dir: Some(custom_dir),
        }
    }

    /// Load a template by name.
    ///
    /// Resolution order:
    /// 1. Built-in embedded templates (case-insensitive name match).
    /// 2. `custom/` prefix → look up in `custom_dir` (or default project-local/global custom dir).
    /// 3. Plain name → try `custom_dir` directly.
    pub fn load(&self, name: &str) -> Result<SimulationTemplate, SimulationError> {
        // 1. Built-in
        for (builtin_name, content) in builtin_templates() {
            if builtin_name.eq_ignore_ascii_case(name) {
                return toml::from_str(content).map_err(|e| SimulationError::ParseError {
                    path: PathBuf::from(format!("<builtin:{builtin_name}>")),
                    reason: e.to_string(),
                });
            }
        }

        // 2. Custom prefix
        let lookup_name = name.strip_prefix("custom/").unwrap_or(name);
        let custom_dir = self.custom_dir.clone().unwrap_or_else(default_custom_dir);

        let candidates = [
            custom_dir.join(format!("{lookup_name}.toml")),
            custom_dir.join(lookup_name),
        ];

        for path in &candidates {
            if path.exists() {
                let content =
                    std::fs::read_to_string(path).map_err(|e| SimulationError::ParseError {
                        path: path.clone(),
                        reason: e.to_string(),
                    })?;
                return toml::from_str(&content).map_err(|e| SimulationError::ParseError {
                    path: path.clone(),
                    reason: e.to_string(),
                });
            }
        }

        Err(SimulationError::TemplateNotFound {
            name: name.to_string(),
        })
    }

    /// List all available template names.
    ///
    /// Returns built-in names first, then `custom/<filename_stem>` for every
    /// `.toml` file found in `custom_dir`.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = builtin_templates()
            .into_iter()
            .map(|(n, _)| n.to_string())
            .collect();

        let custom_dir = self.custom_dir.clone().unwrap_or_else(default_custom_dir);

        if let Ok(entries) = std::fs::read_dir(&custom_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        names.push(format!("custom/{stem}"));
                    }
                }
            }
        }

        names
    }
}

fn agent007_home() -> PathBuf {
    if let Ok(p) = std::env::var("AGENT007_HOME") {
        return PathBuf::from(p);
    }
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        let candidate = dir.join(".agent007");
        if candidate.is_dir() {
            return candidate;
        }
        if !dir.pop() {
            break;
        }
    }
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".agent007")
}

fn default_custom_dir() -> PathBuf {
    agent007_home().join("simulations").join("custom")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn loader_finds_builtin_wifi_rtt() {
        let loader = TemplateLoader::new_builtin_only();
        let t = loader.load("wifi-rtt").unwrap();
        assert_eq!(t.name, "wifi-rtt");
        assert!(!t.scenarios.is_empty());
    }

    #[test]
    fn loader_finds_builtin_wifi_roaming() {
        let loader = TemplateLoader::new_builtin_only();
        let t = loader.load("wifi-roaming").unwrap();
        assert_eq!(t.name, "wifi-roaming");
    }

    #[test]
    fn loader_lists_builtins() {
        let loader = TemplateLoader::new_builtin_only();
        let names = loader.list();
        assert!(names.contains(&"wifi-rtt".to_string()));
        assert!(names.contains(&"wifi-roaming".to_string()));
    }

    #[test]
    fn loader_returns_not_found_for_unknown() {
        let loader = TemplateLoader::new_builtin_only();
        let err = loader.load("does-not-exist").unwrap_err();
        assert!(matches!(err, SimulationError::TemplateNotFound { .. }));
    }

    #[test]
    fn loader_finds_custom_template() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("gps-urban.toml"),
            r#"
name = "gps-urban"
description = "GPS urban canyon test"
research_topics = []
[system_under_test]
command = "echo"
[[scenarios]]
name = "downtown"
"#,
        )
        .unwrap();

        let loader = TemplateLoader::with_custom_dir(tmp.path().to_path_buf());
        let t = loader.load("custom/gps-urban").unwrap();
        assert_eq!(t.name, "gps-urban");
    }

    #[test]
    fn loader_lists_custom_templates() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("my-sim.toml"),
            "[system_under_test]\ncommand=\"echo\"\nname=\"my-sim\"\nresearch_topics=[]",
        )
        .unwrap();

        let loader = TemplateLoader::with_custom_dir(tmp.path().to_path_buf());
        let names = loader.list();
        assert!(names.contains(&"custom/my-sim".to_string()));
    }
}
