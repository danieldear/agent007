# Personas Crate Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `agent007-personas` crate providing 10 built-in specialist agent personas with a `PersonaRegistry` that implements the `PersonaProvider` trait already defined in `core`.

**Architecture:** New `crates/personas` crate implements `PersonaProvider` trait from `agent007-core`. The `cli` crate loads `PersonaRegistry` and passes it as `Arc<dyn PersonaProvider>` to `Orchestrator`, replacing the existing `NoOpPersonaProvider`. Built-in personas are hardcoded in Rust; user overrides load from `~/.agent007/personas/*.toml`.

**Tech Stack:** Rust, thiserror, serde/toml, agent007-core (PersonaProvider trait)

---

## File Structure

```
crates/personas/
├── Cargo.toml
└── src/
    ├── lib.rs          # pub re-exports: PersonaRegistry, PersonaError
    ├── error.rs        # PersonaError (thiserror)
    ├── registry.rs     # PersonaRegistry + all 10 built-in persona definitions
    └── loader.rs       # user override loading from ~/.agent007/personas/*.toml

crates/core/src/
    ├── lib.rs          # add: pub mod persona; pub use persona::{PersonaSpec, PersonaProvider, NoOpPersonaProvider}
    └── persona.rs      # NEW: PersonaSpec, PersonaProvider trait, NoOpPersonaProvider

crates/cli/src/
    ├── main.rs         # add PersonaArgs + PersonaAction, add Commands::Persona arm
    ├── commands/
    │   ├── mod.rs      # add: pub mod persona
    │   ├── persona.rs  # NEW: execute(config, action) for persona list/show
    │   ├── run.rs      # wire PersonaRegistry into build_stack (replace NoOpPersonaProvider)
    │   └── serve.rs    # add agent007_persona_list + agent007_persona_show MCP tools

Modify: Cargo.toml (workspace root — add crates/personas to members)
Modify: crates/cli/Cargo.toml (add agent007-personas dep)
```

---

## Task 1: Add PersonaSpec + PersonaProvider to core

**Files:**
- Create: `crates/core/src/persona.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod persona` + re-exports)

- [ ] **Step 1.1: Write failing test in core for PersonaProvider trait**

Add a test module at the bottom of the new `crates/core/src/persona.rs` that verifies the trait contract compiles and `NoOpPersonaProvider` returns the expected values:

```rust
// crates/core/src/persona.rs  (write the test block first, before any impl)
#[cfg(test)]
mod tests {
    // This will fail to compile until PersonaSpec, PersonaProvider, NoOpPersonaProvider exist.
    use super::*;

    #[test]
    fn noop_provider_returns_none_and_empty() {
        let p = NoOpPersonaProvider;
        assert!(p.get("Researcher").is_none());
        assert!(p.list().is_empty());
    }

    #[test]
    fn persona_spec_fields_are_accessible() {
        let spec = PersonaSpec {
            name: "Test".to_string(),
            description: "desc".to_string(),
            system_prompt: "you are...".to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec!["bash".to_string()],
        };
        assert_eq!(spec.name, "Test");
        assert_eq!(spec.preferred_model, "claude");
        assert_eq!(spec.allowed_tools.len(), 1);
    }
}
```

Run (expect compile failure — types don't exist yet):
```
cargo test -p agent007-core 2>&1 | head -20
```

- [ ] **Step 1.2: Implement PersonaSpec, PersonaProvider, NoOpPersonaProvider**

Create `crates/core/src/persona.rs`:

```rust
// crates/core/src/persona.rs
use serde::{Deserialize, Serialize};

/// Full specification for a single persona.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaSpec {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub preferred_model: String,
    pub allowed_tools: Vec<String>,
}

/// Trait for any store of personas.  Must be Send + Sync so it can be held in Arc<dyn PersonaProvider>.
pub trait PersonaProvider: Send + Sync {
    /// Look up a persona by exact name (case-sensitive).
    fn get(&self, name: &str) -> Option<PersonaSpec>;

    /// List all available personas.
    fn list(&self) -> Vec<PersonaSpec>;
}

/// Stub implementation — always returns None / empty.  Used before PersonaRegistry is wired in.
pub struct NoOpPersonaProvider;

impl PersonaProvider for NoOpPersonaProvider {
    fn get(&self, _name: &str) -> Option<PersonaSpec> {
        None
    }

    fn list(&self) -> Vec<PersonaSpec> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_provider_returns_none_and_empty() {
        let p = NoOpPersonaProvider;
        assert!(p.get("Researcher").is_none());
        assert!(p.list().is_empty());
    }

    #[test]
    fn persona_spec_fields_are_accessible() {
        let spec = PersonaSpec {
            name: "Test".to_string(),
            description: "desc".to_string(),
            system_prompt: "you are...".to_string(),
            preferred_model: "claude".to_string(),
            allowed_tools: vec!["bash".to_string()],
        };
        assert_eq!(spec.name, "Test");
        assert_eq!(spec.preferred_model, "claude");
        assert_eq!(spec.allowed_tools.len(), 1);
    }
}
```

- [ ] **Step 1.3: Wire persona module into core lib.rs**

In `crates/core/src/lib.rs`, add after the existing `pub mod types;` line:

```rust
pub mod persona;

pub use persona::{PersonaSpec, PersonaProvider, NoOpPersonaProvider};
```

Also add `serde = { workspace = true }` to `crates/core/Cargo.toml` `[dependencies]` if not already present.

- [ ] **Step 1.4: Run tests — expect green**

```
cargo test -p agent007-core
```

- [ ] **Step 1.5: Commit**

```
git add crates/core/src/persona.rs crates/core/src/lib.rs crates/core/Cargo.toml
git commit -m "feat(core): add PersonaSpec, PersonaProvider trait, NoOpPersonaProvider"
```

---

## Task 2: Scaffold personas crate

**Files:**
- Create: `crates/personas/Cargo.toml`
- Create: `crates/personas/src/lib.rs`
- Modify: `Cargo.toml` (workspace root — add `crates/personas`)

- [ ] **Step 2.1: Write failing smoke test**

Create `crates/personas/src/lib.rs` with just the test (will fail to compile because the crate doesn't exist in workspace yet):

```rust
// crates/personas/src/lib.rs  (temporary placeholder — will be replaced in Step 2.4)
pub mod error;
pub mod registry;
pub mod loader;

pub use error::PersonaError;
pub use registry::PersonaRegistry;

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::PersonaProvider;

    #[test]
    fn built_in_returns_non_empty_list() {
        let registry = PersonaRegistry::built_in();
        assert!(!registry.list().is_empty());
    }
}
```

Run (expect error — crate not in workspace):
```
cargo build -p agent007-personas 2>&1 | head -10
```

- [ ] **Step 2.2: Add personas to workspace Cargo.toml**

In the root `Cargo.toml`, add `"crates/personas"` to the `[workspace] members` array:

```toml
[workspace]
members = [
    "crates/cli",
    "crates/models",
    "crates/core",
    "crates/memory",
    "crates/skills",
    "crates/hooks",
    "crates/mcp",
    "crates/learning",
    "crates/tui",
    "crates/personas",
]
```

- [ ] **Step 2.3: Create crates/personas/Cargo.toml**

```toml
# crates/personas/Cargo.toml
[package]
name = "agent007-personas"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core = { path = "../core" }
thiserror = { workspace = true }
serde = { workspace = true }
toml = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 2.4: Create minimal stub files so crate compiles**

Create `crates/personas/src/error.rs` (stub — will be replaced in Task 3):

```rust
// crates/personas/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum PersonaError {
    #[error("IO error reading persona dir: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse persona file {path}: {reason}")]
    ParseError { path: std::path::PathBuf, reason: String },
}
```

Create `crates/personas/src/registry.rs` (stub — will be replaced in Task 4):

```rust
// crates/personas/src/registry.rs
use std::collections::HashMap;
use agent007_core::{PersonaProvider, PersonaSpec};
use crate::error::PersonaError;

pub struct PersonaRegistry {
    personas: HashMap<String, PersonaSpec>,
}

impl PersonaRegistry {
    pub fn built_in() -> Self {
        Self { personas: HashMap::new() }
    }

    pub fn load(_user_dir: &std::path::Path) -> Result<Self, PersonaError> {
        Ok(Self::built_in())
    }
}

impl PersonaProvider for PersonaRegistry {
    fn get(&self, name: &str) -> Option<PersonaSpec> {
        self.personas.get(name).cloned()
    }

    fn list(&self) -> Vec<PersonaSpec> {
        self.personas.values().cloned().collect()
    }
}
```

Create `crates/personas/src/loader.rs` (stub — will be replaced in Task 5):

```rust
// crates/personas/src/loader.rs
// User override loading — implemented in Task 5.
```

- [ ] **Step 2.5: Verify crate compiles (test will fail on the assertion — that is expected)**

```
cargo build -p agent007-personas
cargo test -p agent007-personas 2>&1
```

The `built_in_returns_non_empty_list` test should fail (empty list). That is the red phase.

- [ ] **Step 2.6: Commit scaffold**

```
git add crates/personas/ Cargo.toml
git commit -m "feat(personas): scaffold personas crate with stub registry"
```

---

## Task 3: PersonaError type

**Files:**
- Modify: `crates/personas/src/error.rs` (already created in Task 2 — verify it is final)

The error type written in Step 2.4 is already the final implementation. Verify it handles both variants correctly with a test.

- [ ] **Step 3.1: Write failing tests for PersonaError**

Add a `#[cfg(test)]` block to `crates/personas/src/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn io_error_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let err = PersonaError::Io(io_err);
        let msg = err.to_string();
        assert!(msg.contains("IO error reading persona dir"));
    }

    #[test]
    fn parse_error_display_contains_path_and_reason() {
        let err = PersonaError::ParseError {
            path: PathBuf::from("/home/.agent007/personas/bad.toml"),
            reason: "missing field `name`".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("bad.toml"));
        assert!(msg.contains("missing field"));
    }
}
```

Run (expect green — error type is already implemented):
```
cargo test -p agent007-personas error
```

- [ ] **Step 3.2: Commit**

```
git add crates/personas/src/error.rs
git commit -m "test(personas): add PersonaError unit tests"
```

---

## Task 4: Built-in persona definitions

**Files:**
- Modify: `crates/personas/src/registry.rs` (replace stub with full implementation)

- [ ] **Step 4.1: Write failing tests for built-in personas**

Add a `#[cfg(test)]` block to `crates/personas/src/registry.rs` (before the final impl, will be read after the impl replaces the stub):

```rust
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
}
```

Run (expect red — built_in returns empty HashMap):
```
cargo test -p agent007-personas registry
```

- [ ] **Step 4.2: Implement built-in persona definitions**

Replace `crates/personas/src/registry.rs` with the full implementation:

```rust
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
}
```

- [ ] **Step 4.3: Run registry tests — expect green**

```
cargo test -p agent007-personas registry
```

- [ ] **Step 4.4: Run full crate tests**

```
cargo test -p agent007-personas
```

- [ ] **Step 4.5: Commit**

```
git add crates/personas/src/registry.rs
git commit -m "feat(personas): implement 10 built-in persona definitions in PersonaRegistry"
```

---

## Task 5: User override loading from TOML

**Files:**
- Modify: `crates/personas/src/loader.rs` (replace stub with full implementation)

- [ ] **Step 5.1: Write failing tests for user override loading**

Replace the stub `crates/personas/src/loader.rs` with tests + empty function signature:

```rust
// crates/personas/src/loader.rs
use std::path::Path;
use agent007_core::PersonaSpec;
use crate::error::PersonaError;

/// Load all PersonaSpec overrides from *.toml files in user_dir.
/// Files that fail to parse return PersonaError::ParseError.
/// Non-.toml files are silently ignored.
pub fn load_user_overrides(user_dir: &Path) -> Result<Vec<PersonaSpec>, PersonaError> {
    todo!("implement in Step 5.2")
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
        assert_eq!(specs[0].allowed_tools, vec!["bash", "file_read", "file_write"]);
    }

    #[test]
    fn ignores_non_toml_files() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("notes.txt"),
            "not a toml persona",
        )
        .unwrap();
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
        assert!(msg.contains("bad.toml"), "error should mention the file path");
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
```

Run (expect panic/compile error from `todo!()`):
```
cargo test -p agent007-personas loader 2>&1 | head -30
```

- [ ] **Step 5.2: Implement load_user_overrides**

Replace the `todo!()` body in `crates/personas/src/loader.rs`:

```rust
// crates/personas/src/loader.rs
use std::path::Path;
use agent007_core::PersonaSpec;
use crate::error::PersonaError;

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

// (tests block from Step 5.1 remains here unchanged)
```

- [ ] **Step 5.3: Run loader tests — expect green**

```
cargo test -p agent007-personas loader
```

- [ ] **Step 5.4: Run full crate tests**

```
cargo test -p agent007-personas
```

- [ ] **Step 5.5: Commit**

```
git add crates/personas/src/loader.rs
git commit -m "feat(personas): implement user override loading from ~/.agent007/personas/*.toml"
```

---

## Task 6: PersonaRegistry::load combining built-ins + user overrides

**Files:**
- Modify: `crates/personas/src/registry.rs` (add integration tests for `load()`)

The `load()` method was already implemented in Task 4 Step 4.2. This task adds integration tests that exercise the full round-trip.

- [ ] **Step 6.1: Write integration tests for PersonaRegistry::load**

Add the following tests to the `#[cfg(test)]` block in `crates/personas/src/registry.rs`:

```rust
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
```

Also add `use tempfile::TempDir;` inside the test module (add `tempfile = { workspace = true }` to `[dev-dependencies]` in `crates/personas/Cargo.toml` if not already present — it was added in Task 2).

- [ ] **Step 6.2: Run integration tests — expect green**

```
cargo test -p agent007-personas
```

- [ ] **Step 6.3: Run full workspace tests**

```
cargo test
```

- [ ] **Step 6.4: Commit**

```
git add crates/personas/src/registry.rs
git commit -m "test(personas): add PersonaRegistry::load integration tests"
```

---

## Task 7: Wire PersonaRegistry into CLI build_stack

**Files:**
- Modify: `crates/cli/Cargo.toml` (add agent007-personas dep)
- Modify: `crates/cli/src/commands/run.rs` (add persona_registry to Stack, wire into build_stack)

- [ ] **Step 7.1: Write failing test for build_stack containing persona_registry**

Add a test in `crates/cli/src/commands/run.rs` (inside the existing `tests` module):

```rust
    #[tokio::test]
    async fn build_stack_contains_persona_registry_with_builtins() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Config::default();
        let stack = build_stack(&config).await.unwrap();
        // PersonaRegistry must expose at least 10 built-in personas
        use agent007_core::PersonaProvider;
        assert!(stack.persona_registry.list().len() >= 10);
        std::env::remove_var("AGENT007_DRY_RUN");
    }
```

Run (expect compile error — `persona_registry` field doesn't exist yet):
```
cargo test -p agent007 commands::run::tests::build_stack_contains_persona_registry_with_builtins 2>&1 | head -20
```

- [ ] **Step 7.2: Add agent007-personas to crates/cli/Cargo.toml**

In `crates/cli/Cargo.toml`, add to `[dependencies]`:

```toml
agent007-personas = { path = "../personas" }
```

- [ ] **Step 7.3: Add persona_registry field to Stack and wire build_stack**

In `crates/cli/src/commands/run.rs`:

1. Add import at the top:
```rust
use agent007_personas::PersonaRegistry;
```

2. Add field to `Stack`:
```rust
pub struct Stack {
    // ... existing fields ...
    pub persona_registry: Arc<PersonaRegistry>,
}
```

3. In `build_stack`, after step 10 (SkillExecutor), add step 11 before OrchestratorAgent:
```rust
    // 11. PersonaRegistry — load built-ins + user overrides from ~/.agent007/personas/
    let personas_dir = home.join("personas");
    let persona_registry = Arc::new(
        PersonaRegistry::load(&personas_dir).unwrap_or_else(|e| {
            tracing::warn!("failed to load persona overrides from {}: {}", personas_dir.display(), e);
            PersonaRegistry::built_in()
        })
    );
```

4. Add `persona_registry` to the returned `Stack { ... }` struct literal.

5. Renumber the OrchestratorAgent step comment from 11 to 12.

- [ ] **Step 7.4: Run test — expect green**

```
cargo test -p agent007 commands::run::tests
```

- [ ] **Step 7.5: Run full workspace**

```
cargo test
```

- [ ] **Step 7.6: Commit**

```
git add crates/cli/Cargo.toml crates/cli/src/commands/run.rs
git commit -m "feat(cli): wire PersonaRegistry into build_stack, expose on Stack"
```

---

## Task 8: agent007 persona list/show CLI commands

**Files:**
- Create: `crates/cli/src/commands/persona.rs`
- Modify: `crates/cli/src/commands/mod.rs` (add `pub mod persona`)
- Modify: `crates/cli/src/main.rs` (add `PersonaArgs`, `PersonaAction`, `Commands::Persona` arm)

- [ ] **Step 8.1: Write failing tests for persona CLI commands**

Create `crates/cli/src/commands/persona.rs` with tests first:

```rust
// crates/cli/src/commands/persona.rs
use std::sync::Arc;
use anyhow::Result;
use agent007_core::PersonaProvider;
use agent007_personas::PersonaRegistry;
use crate::PersonaAction;

/// Top-level dispatch for `agent007 persona <action>`.
pub async fn execute(_config: Arc<crate::config::Config>, action: PersonaAction) -> Result<()> {
    todo!("implement in Step 8.3")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_output_covers_all_ten_builtins() {
        // We test the data layer directly — formatting is handled separately.
        let registry = PersonaRegistry::built_in();
        let personas = registry.list();
        assert_eq!(personas.len(), 10);
        let names: Vec<&str> = personas.iter().map(|p| p.name.as_str()).collect();
        for expected in &[
            "Researcher", "Architect", "Coder", "TestDesigner",
            "SecurityReviewer", "PerformanceEngineer", "DocumentationWriter",
            "DependencyManager", "DebugAgent", "CodeReviewer",
        ] {
            assert!(names.contains(expected), "missing persona in list: {}", expected);
        }
    }

    #[test]
    fn show_known_persona_returns_spec() {
        let registry = PersonaRegistry::built_in();
        let spec = registry.get("Researcher");
        assert!(spec.is_some());
        let spec = spec.unwrap();
        assert!(!spec.system_prompt.is_empty());
    }

    #[test]
    fn show_unknown_persona_returns_none() {
        let registry = PersonaRegistry::built_in();
        let spec = registry.get("Ghost");
        assert!(spec.is_none());
    }
}
```

Run (expect todo! panic):
```
cargo test -p agent007 commands::persona::tests 2>&1 | head -20
```

- [ ] **Step 8.2: Add PersonaArgs and PersonaAction to main.rs**

In `crates/cli/src/main.rs`:

1. Add to the `Commands` enum:
```rust
    /// Manage personas
    Persona(PersonaArgs),
```

2. Add new structs after `SkillArgs`:
```rust
#[derive(Parser, Debug)]
pub struct PersonaArgs {
    #[command(subcommand)]
    pub action: PersonaAction,
}

#[derive(Subcommand, Debug)]
pub enum PersonaAction {
    /// List all available personas (built-in + user overrides)
    List,
    /// Show full details (system prompt) for a named persona
    Show {
        /// Exact persona name, e.g. Researcher
        name: String,
    },
}
```

3. Add the match arm in `main()`:
```rust
        Commands::Persona(p) => commands::persona::execute(config, p.action).await,
```

4. Add parse tests:
```rust
    #[test]
    fn parse_persona_list_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "persona", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Persona(ref p) if matches!(p.action, PersonaAction::List)
        ));
    }

    #[test]
    fn parse_persona_show_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "persona", "show", "Researcher"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Persona(ref p) if matches!(p.action, PersonaAction::Show { ref name } if name == "Researcher")
        ));
    }
```

- [ ] **Step 8.3: Implement execute in persona.rs**

Replace the `todo!()` in `crates/cli/src/commands/persona.rs`:

```rust
// crates/cli/src/commands/persona.rs
use std::sync::Arc;
use anyhow::Result;
use agent007_core::PersonaProvider;
use agent007_personas::PersonaRegistry;
use crate::PersonaAction;

/// Top-level dispatch for `agent007 persona <action>`.
pub async fn execute(_config: Arc<crate::config::Config>, action: PersonaAction) -> Result<()> {
    let personas_dir = crate::commands::run::agent007_home().join("personas");
    let registry = PersonaRegistry::load(&personas_dir).unwrap_or_else(|e| {
        tracing::warn!("failed to load persona overrides: {}", e);
        PersonaRegistry::built_in()
    });

    match action {
        PersonaAction::List => {
            let personas = registry.list();
            if personas.is_empty() {
                println!("No personas available.");
            } else {
                println!("{:<22} {:<10} {}", "NAME", "MODEL", "DESCRIPTION");
                println!("{}", "-".repeat(72));
                for p in personas {
                    println!("{:<22} {:<10} {}", p.name, p.preferred_model, p.description);
                }
            }
        }
        PersonaAction::Show { name } => {
            match registry.get(&name) {
                Some(spec) => {
                    println!("Name:            {}", spec.name);
                    println!("Model:           {}", spec.preferred_model);
                    println!("Description:     {}", spec.description);
                    if !spec.allowed_tools.is_empty() {
                        println!("Allowed tools:   {}", spec.allowed_tools.join(", "));
                    }
                    println!();
                    println!("System prompt:");
                    println!("{}", spec.system_prompt);
                }
                None => {
                    anyhow::bail!("persona '{}' not found. Run `agent007 persona list` to see available personas.", name);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_output_covers_all_ten_builtins() {
        let registry = PersonaRegistry::built_in();
        let personas = registry.list();
        assert_eq!(personas.len(), 10);
        let names: Vec<&str> = personas.iter().map(|p| p.name.as_str()).collect();
        for expected in &[
            "Researcher", "Architect", "Coder", "TestDesigner",
            "SecurityReviewer", "PerformanceEngineer", "DocumentationWriter",
            "DependencyManager", "DebugAgent", "CodeReviewer",
        ] {
            assert!(names.contains(expected), "missing persona in list: {}", expected);
        }
    }

    #[test]
    fn show_known_persona_returns_spec() {
        let registry = PersonaRegistry::built_in();
        let spec = registry.get("Researcher");
        assert!(spec.is_some());
        let spec = spec.unwrap();
        assert!(!spec.system_prompt.is_empty());
    }

    #[test]
    fn show_unknown_persona_returns_none() {
        let registry = PersonaRegistry::built_in();
        let spec = registry.get("Ghost");
        assert!(spec.is_none());
    }
}
```

- [ ] **Step 8.4: Add pub mod persona to commands/mod.rs**

In `crates/cli/src/commands/mod.rs`:

```rust
pub mod run;
pub mod serve;
pub mod skill;
pub mod simulate;
pub mod persona;
```

- [ ] **Step 8.5: Run all CLI tests**

```
cargo test -p agent007
```

- [ ] **Step 8.6: Run full workspace**

```
cargo test
```

- [ ] **Step 8.7: Commit**

```
git add crates/cli/src/commands/persona.rs crates/cli/src/commands/mod.rs crates/cli/src/main.rs
git commit -m "feat(cli): add 'agent007 persona list' and 'agent007 persona show' commands"
```

---

## Task 9: Add persona MCP tools to serve.rs

**Files:**
- Modify: `crates/cli/src/commands/serve.rs` (add `agent007_persona_list` and `agent007_persona_show` tools)

- [ ] **Step 9.1: Write failing tests for MCP persona tools**

Add to the bottom of `crates/cli/src/commands/serve.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_defs_contains_persona_list_and_show() {
        let defs = Agent007Server::tool_defs();
        let names: Vec<&str> = defs.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"agent007_persona_list"), "missing agent007_persona_list");
        assert!(names.contains(&"agent007_persona_show"), "missing agent007_persona_show");
    }
}
```

Run (expect failure — tools not in defs yet):
```
cargo test -p agent007 commands::serve::tests 2>&1 | head -20
```

- [ ] **Step 9.2: Add persona tools to tool_defs()**

In `Agent007Server::tool_defs()`, append after the existing `agent007_skill_run` tool:

```rust
            tool(
                "agent007_persona_list",
                "List all available agent007 personas (built-in + user overrides from ~/.agent007/personas/). \
                 Returns name, preferred model, and description for each persona.",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),
            tool(
                "agent007_persona_show",
                "Show full details (including system prompt and allowed tools) for a named agent007 persona.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Exact persona name, e.g. Researcher"
                        }
                    },
                    "required": ["name"]
                }),
            ),
```

- [ ] **Step 9.3: Add arms to call_tool()**

In `Agent007Server::call_tool()`, add after the `"agent007_skill_run"` arm (before the final `name =>` catch-all):

```rust
            "agent007_persona_list" => {
                let personas_dir = agent007_home().join("personas");
                let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
                    .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
                use agent007_core::PersonaProvider;
                let personas = registry.list();
                let text = if personas.is_empty() {
                    "No personas available.".to_string()
                } else {
                    personas
                        .iter()
                        .map(|p| format!("• {} [{}] — {}", p.name, p.preferred_model, p.description))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            "agent007_persona_show" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let personas_dir = agent007_home().join("personas");
                let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
                    .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
                use agent007_core::PersonaProvider;
                match registry.get(&name) {
                    Some(spec) => {
                        let tools = if spec.allowed_tools.is_empty() {
                            "none".to_string()
                        } else {
                            spec.allowed_tools.join(", ")
                        };
                        let text = format!(
                            "Name: {}\nModel: {}\nDescription: {}\nAllowed tools: {}\n\nSystem prompt:\n{}",
                            spec.name, spec.preferred_model, spec.description, tools, spec.system_prompt
                        );
                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                    None => Ok(CallToolResult::error(vec![Content::text(
                        format!("Persona '{}' not found.", name)
                    )])),
                }
            }
```

- [ ] **Step 9.4: Add use import for agent007_personas in serve.rs**

At the top of `crates/cli/src/commands/serve.rs`, add:

```rust
// (no extra use needed — crate is accessed via full path agent007_personas::... above)
// If preferred, add: use agent007_personas::PersonaRegistry;
```

- [ ] **Step 9.5: Run serve tests — expect green**

```
cargo test -p agent007 commands::serve::tests
```

- [ ] **Step 9.6: Run full workspace**

```
cargo test
```

- [ ] **Step 9.7: Commit**

```
git add crates/cli/src/commands/serve.rs
git commit -m "feat(cli/serve): add agent007_persona_list and agent007_persona_show MCP tools"
```

---

## Final Verification

- [ ] **Full workspace clean build and test**

```
cargo build
cargo test
```

- [ ] **Verify binary help shows persona subcommand**

```
cargo run -p agent007 -- --help
cargo run -p agent007 -- persona list
cargo run -p agent007 -- persona show Researcher
```

- [ ] **Verify all 10 personas appear in list output with correct names and models**

Expected output includes:
```
Architect          claude     System design, trade-offs, and API contract definition
Coder              codex      Implementation, refactoring, and code generation
CodeReviewer       claude     Code quality, style consistency, and architectural review
DebugAgent         claude     Error analysis, failure diagnosis, and fix proposals
DependencyManager  claude     CVE scanning, version updates, and compatibility checks
DocumentationWriter claude    Docstrings, READMEs, changelogs, and API documentation
PerformanceEngineer claude    Profiling, bottleneck identification, and algorithmic complexity
Researcher         claude     Web search, documentation, standards, and best practices
SecurityReviewer   claude     OWASP vulnerabilities, authentication, secrets scanning
TestDesigner       codex      TDD, edge case enumeration, and coverage analysis
```

- [ ] **Final commit if any cleanup needed**

```
git add -p
git commit -m "chore(personas): final cleanup and verification"
```
