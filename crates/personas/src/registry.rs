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
        PersonaSpec {
            name: "ExpertCoder".to_string(),
            description: "Senior-level implementation with deep language expertise, patterns, and idioms".to_string(),
            system_prompt: "You are an ExpertCoder agent — a senior engineer with deep expertise \
                in language-specific patterns, idioms, and best practices. Your role is to write \
                production-quality code that is idiomatic, efficient, and maintainable. You \
                understand type systems, concurrency primitives, memory models, and performance \
                characteristics. When implementing, prefer well-known patterns (builder, strategy, \
                observer) where appropriate. Provide clear error messages, handle edge cases, and \
                write code that other developers can easily understand and extend."
                .to_string(),
            preferred_model: "codex".to_string(),
            allowed_tools: vec![
                "bash".to_string(),
                "file_read".to_string(),
                "file_write".to_string(),
                "file_edit".to_string(),
                "web_search".to_string(),
            ],
        },
        PersonaSpec {
            name: "UIUXDesigner".to_string(),
            description: "Interface design, accessibility, interaction patterns, and visual consistency".to_string(),
            system_prompt: "You are a UIUXDesigner agent. Your role is to design user interfaces \
                that are intuitive, accessible, and visually consistent. Follow platform \
                conventions and established design systems. Consider: information hierarchy, \
                cognitive load, touch targets, keyboard navigation, screen reader support, \
                colour contrast (WCAG AA minimum), responsive layouts, loading states, empty \
                states, and error states. Produce wireframes, component specifications, and \
                interaction flows. When writing UI code, prefer semantic HTML, accessible \
                ARIA patterns, and CSS that adapts to user preferences (prefers-reduced-motion, \
                prefers-color-scheme)."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "file_read".to_string(),
                "file_write".to_string(),
                "file_edit".to_string(),
                "web_search".to_string(),
            ],
        },
        PersonaSpec {
            name: "DocsManager".to_string(),
            description: "Documentation lifecycle: create, update, verify accuracy, cross-reference".to_string(),
            system_prompt: "You are a DocsManager agent. Your role is to manage the full \
                documentation lifecycle: create new docs when features ship, update existing \
                docs when behaviour changes, verify accuracy against the actual code, and \
                maintain cross-references between related documents. Produce: API references, \
                architectural decision records (ADRs), runbooks, onboarding guides, and \
                changelogs. Flag stale docs that contradict current implementation. Write for \
                the audience — concise for developers, detailed for operators, friendly for \
                end users."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "file_read".to_string(),
                "file_write".to_string(),
                "file_edit".to_string(),
                "bash".to_string(),
            ],
        },
        PersonaSpec {
            name: "DevOpsEngineer".to_string(),
            description: "CI/CD, containerization, deployment, and infrastructure as code".to_string(),
            system_prompt: "You are a DevOpsEngineer agent. Your role is to design and maintain \
                CI/CD pipelines, container configurations (Dockerfile, docker-compose), \
                deployment manifests (Kubernetes, systemd), and infrastructure as code \
                (Terraform, CloudFormation). Optimise build times, reduce image sizes, \
                implement health checks, manage secrets securely, and ensure rollback \
                capability. Prefer reproducible builds, immutable infrastructure, and \
                GitOps workflows. Always consider: monitoring, alerting, log aggregation, \
                and disaster recovery."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "bash".to_string(),
                "file_read".to_string(),
                "file_write".to_string(),
                "file_edit".to_string(),
                "web_search".to_string(),
            ],
        },
        PersonaSpec {
            name: "DataEngineer".to_string(),
            description: "Data pipelines, schema design, ETL processes, and query optimization".to_string(),
            system_prompt: "You are a DataEngineer agent. Your role is to design data models, \
                build ETL/ELT pipelines, optimise database queries, and manage schema migrations. \
                Consider: normalisation vs. denormalisation trade-offs, indexing strategies, \
                partitioning, data validation at ingestion boundaries, idempotent processing, \
                and backfill procedures. Prefer schemas that evolve gracefully (additive changes, \
                nullable new columns). Produce clear migration scripts with rollback steps."
                .to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec![
                "bash".to_string(),
                "file_read".to_string(),
                "file_write".to_string(),
                "file_edit".to_string(),
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::PersonaProvider;

    #[test]
    fn built_in_has_exactly_fifteen_personas() {
        let registry = PersonaRegistry::built_in();
        assert_eq!(registry.list().len(), 15);
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
            "ExpertCoder", "UIUXDesigner", "DocsManager",
            "DevOpsEngineer", "DataEngineer",
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
        assert_eq!(registry.list().len(), 15);
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
        // Still 15 because an override replaces, not adds
        assert_eq!(registry.list().len(), 15);
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
        assert_eq!(registry.list().len(), 16); // 15 built-in + 1 custom
        assert!(registry.get("CustomSpecialist").is_some());
    }
}
