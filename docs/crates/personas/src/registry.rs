// crates/personas/src/registry.rs
use std::collections::HashMap;
use std::path::Path;
use agent007_core::{PersonaProvider, PersonaSpec};
use crate::error::PersonaError;
use crate::loader::load_user_overrides;

pub struct PersonaRegistry {
    personas: HashMap<String, PersonaSpec>,
}

impl PersonaRegistry {
    /// Load built-in personas plus any user overrides found in user_dir (e.g. ~/.agent007/personas/).
    /// If user_dir does not exist, only built-ins are returned.
    pub fn load(user_dir: &Path) -> Result<Self, PersonaError> {
        let mut registry = Self::built_in();
        if user_dir.exists() {
            let overrides = load_user_overrides(user_dir)?;
            for spec in overrides {
                registry.personas.insert(spec.name.clone(), spec);
            }
        }
        Ok(registry)
    }

    /// Return a registry containing only the 10 built-in personas.
    pub fn built_in() -> Self {
        let mut personas = HashMap::new();
        for spec in builtin_personas() {
            personas.insert(spec.name.clone(), spec);
        }
        Self { personas }
    }
}

impl PersonaProvider for PersonaRegistry {
    fn get(&self, name: &str) -> Option<PersonaSpec> {
        self.personas.get(name).cloned()
    }

    fn list(&self) -> Vec<PersonaSpec> {
        let mut specs: Vec<PersonaSpec> = self.personas.values().cloned().collect();
        // Sort by name for deterministic output
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }
}

// ── Built-in persona definitions ─────────────────────────────────────────────

fn builtin_personas() -> Vec<PersonaSpec> {
    vec![
        PersonaSpec {
            name: "Researcher".to_string(),
            description: "Web search, documentation, standards, and best practices".to_string(),
            system_prompt: "You are a Researcher agent. Your role is to gather information from \
                web searches, official documentation, standards bodies, and community best \
                practices. Always cite sources. Prefer authoritative references. Summarise \
                findings concisely and flag conflicting information. Do not implement code — \
                produce research reports and recommendations."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "web_search".to_string(),
                "web_fetch".to_string(),
                "file_read".to_string(),
            ],
        },
        PersonaSpec {
            name: "Architect".to_string(),
            description: "System design, trade-off analysis, and API contract definition".to_string(),
            system_prompt: "You are an Architect agent. Your role is to design software systems: \
                define module boundaries, data flows, API contracts, and evaluate architectural \
                trade-offs (performance vs. maintainability, coupling vs. cohesion). Produce \
                structured design documents and ADRs. Do not write production code — focus on \
                diagrams, interfaces, and rationale."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "file_read".to_string(),
                "file_write".to_string(),
            ],
        },
        PersonaSpec {
            name: "Coder".to_string(),
            description: "Implementation, refactoring, and code generation".to_string(),
            system_prompt: "You are a Coder agent. Your role is to write, refactor, and optimise \
                production-quality code. Follow the project's language conventions and style guide. \
                Prefer clarity over cleverness. When refactoring, preserve existing behaviour \
                unless explicitly asked to change it. Always run the linter/formatter before \
                considering a task done."
                .to_string(),
            preferred_model: "codex".to_string(),
            allowed_tools: vec![
                "bash".to_string(),
                "file_read".to_string(),
                "file_write".to_string(),
                "file_edit".to_string(),
            ],
        },
        PersonaSpec {
            name: "TestDesigner".to_string(),
            description: "TDD, edge case enumeration, and coverage analysis".to_string(),
            system_prompt: "You are a TestDesigner agent. Your role is to write comprehensive \
                tests using TDD methodology: write the failing test first, then confirm the \
                implementation passes. Enumerate happy paths, error paths, and boundary \
                conditions. Aim for meaningful coverage — not just line coverage but branch \
                and property coverage where applicable. Use the project's testing framework."
                .to_string(),
            preferred_model: "codex".to_string(),
            allowed_tools: vec![
                "bash".to_string(),
                "file_read".to_string(),
                "file_write".to_string(),
                "file_edit".to_string(),
            ],
        },
        PersonaSpec {
            name: "SecurityReviewer".to_string(),
            description: "OWASP vulnerabilities, authentication, secrets scanning".to_string(),
            system_prompt: "You are a SecurityReviewer agent. Your role is to audit code and \
                configuration for security vulnerabilities using OWASP Top 10 and CWE as \
                frameworks. Identify: injection flaws, broken auth, sensitive data exposure, \
                misconfigurations, insecure dependencies, and secrets in source. Produce a \
                prioritised list of findings with severity (Critical/High/Medium/Low), \
                description, and remediation guidance."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "bash".to_string(),
                "file_read".to_string(),
                "web_search".to_string(),
            ],
        },
        PersonaSpec {
            name: "PerformanceEngineer".to_string(),
            description: "Profiling, bottleneck identification, and algorithmic complexity".to_string(),
            system_prompt: "You are a PerformanceEngineer agent. Your role is to identify \
                performance bottlenecks through profiling data, algorithmic complexity analysis, \
                and memory usage patterns. Propose concrete optimisations with expected impact \
                and trade-offs. Always measure before and after. Focus on the critical path — \
                avoid premature optimisation of non-bottleneck code."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "bash".to_string(),
                "file_read".to_string(),
            ],
        },
        PersonaSpec {
            name: "DocumentationWriter".to_string(),
            description: "Docstrings, READMEs, changelogs, and API documentation".to_string(),
            system_prompt: "You are a DocumentationWriter agent. Your role is to produce clear, \
                accurate, and complete documentation: inline docstrings, module-level docs, \
                README files, changelogs (Keep a Changelog format), and API reference guides. \
                Write for the intended audience (developer vs. end-user). Prefer examples over \
                abstract descriptions. Keep docs co-located with code and update them when \
                behaviour changes."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "file_read".to_string(),
                "file_write".to_string(),
                "file_edit".to_string(),
            ],
        },
        PersonaSpec {
            name: "DependencyManager".to_string(),
            description: "CVE scanning, version updates, and compatibility checks".to_string(),
            system_prompt: "You are a DependencyManager agent. Your role is to audit project \
                dependencies: check for known CVEs using public vulnerability databases, \
                identify outdated packages, resolve version conflicts, and validate \
                compatibility constraints. Produce a dependency health report and a prioritised \
                update plan, noting breaking-change risk for each update."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "bash".to_string(),
                "web_search".to_string(),
                "file_read".to_string(),
                "file_write".to_string(),
            ],
        },
        PersonaSpec {
            name: "DebugAgent".to_string(),
            description: "Error analysis, failure diagnosis, and fix proposals".to_string(),
            system_prompt: "You are a DebugAgent. Your role is to diagnose failures: analyse \
                error messages, stack traces, logs, and reproduction steps to identify root \
                causes. Propose the minimal fix that resolves the issue without introducing \
                regressions. Always explain why the bug occurred and how the fix prevents \
                recurrence. When uncertain, add targeted logging to gather more information \
                before proposing a fix."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "bash".to_string(),
                "file_read".to_string(),
                "file_edit".to_string(),
                "web_search".to_string(),
            ],
        },
        PersonaSpec {
            name: "CodeReviewer".to_string(),
            description: "Code quality, style consistency, and architectural review".to_string(),
            system_prompt: "You are a CodeReviewer agent. Your role is to review code changes \
                for correctness, maintainability, style consistency, and architectural fit. \
                Comment on: logic errors, missing tests, API design issues, naming clarity, \
                and deviations from project conventions. Be specific — reference line numbers \
                and suggest concrete alternatives. Distinguish blocking issues from \
                non-blocking suggestions."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "file_read".to_string(),
                "bash".to_string(),
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::PersonaProvider;

    #[test]
    fn built_in_has_exactly_ten_personas() {
        let registry = PersonaRegistry::built_in();
        assert_eq!(registry.list().len(), 10);
    }

    #[test]
    fn researcher_persona_exists_and_uses_claude() {
        let registry = PersonaRegistry::built_in();
        let spec = registry.get("Researcher").expect("Researcher must exist");
        assert_eq!(spec.preferred_model, "claude");
        assert!(!spec.system_prompt.is_empty());
    }

    #[test]
    fn coder_persona_exists_and_uses_codex() {
        let registry = PersonaRegistry::built_in();
        let spec = registry.get("Coder").expect("Coder must exist");
        assert_eq!(spec.preferred_model, "codex");
    }

    #[test]
    fn test_designer_persona_exists_and_uses_codex() {
        let registry = PersonaRegistry::built_in();
        let spec = registry.get("TestDesigner").expect("TestDesigner must exist");
        assert_eq!(spec.preferred_model, "codex");
    }

    #[test]
    fn all_personas_have_non_empty_system_prompt_and_description() {
        let registry = PersonaRegistry::built_in();
        for spec in registry.list() {
            assert!(!spec.system_prompt.is_empty(), "empty system_prompt for {}", spec.name);
            assert!(!spec.description.is_empty(), "empty description for {}", spec.name);
        }
    }

    #[test]
    fn get_unknown_persona_returns_none() {
        let registry = PersonaRegistry::built_in();
        assert!(registry.get("NonExistent").is_none());
    }

    #[test]
    fn list_returns_all_expected_names() {
        let registry = PersonaRegistry::built_in();
        let names: Vec<String> = registry.list().into_iter().map(|p| p.name).collect();
        for expected in &[
            "Researcher", "Architect", "Coder", "TestDesigner",
            "SecurityReviewer", "PerformanceEngineer", "DocumentationWriter",
            "DependencyManager", "DebugAgent", "CodeReviewer",
        ] {
            assert!(names.contains(&expected.to_string()), "missing persona: {}", expected);
        }
    }

    #[test]
    fn list_is_sorted_by_name() {
        let registry = PersonaRegistry::built_in();
        let names: Vec<String> = registry.list().into_iter().map(|p| p.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn load_from_nonexistent_dir_returns_only_builtins() {
        let path = std::path::PathBuf::from("/tmp/does_not_exist_agent007_personas_test");
        let registry = PersonaRegistry::load(&path).unwrap();
        assert_eq!(registry.list().len(), 10);
    }

    #[test]
    fn load_user_override_replaces_builtin() {
        let dir = tempfile::TempDir::new().unwrap();
        // Override the Coder persona with a different model
        std::fs::write(
            dir.path().join("coder-override.toml"),
            r#"
name = "Coder"
description = "Overridden coder"
system_prompt = "Custom coder prompt."
preferred_model = "claude"
allowed_tools = ["bash"]
"#,
        )
        .unwrap();

        let registry = PersonaRegistry::load(dir.path()).unwrap();
        // Still 10 because an override replaces, not adds
        assert_eq!(registry.list().len(), 10);
        let coder = registry.get("Coder").unwrap();
        assert_eq!(coder.preferred_model, "claude"); // overridden
        assert_eq!(coder.description, "Overridden coder");
    }

    #[test]
    fn load_new_user_persona_adds_to_registry() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("custom.toml"),
            r#"
name = "CustomSpecialist"
description = "My custom specialist"
system_prompt = "You are a custom specialist."
preferred_model = "claude"
allowed_tools = []
"#,
        )
        .unwrap();

        let registry = PersonaRegistry::load(dir.path()).unwrap();
        assert_eq!(registry.list().len(), 11); // 10 built-in + 1 custom
        assert!(registry.get("CustomSpecialist").is_some());
    }
}
