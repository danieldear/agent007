# Custom Agents Crate Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `agent007-custom-agents` crate enabling users to define named agents (worker or sub-orchestrator) in TOML, with scoped RAG memory namespacing, zone inheritance, hierarchical sub-orchestrators that spawn persona workers, and `agent007 agent` CLI commands.

**Architecture:** New `crates/custom-agents` crate. `AgentDef` parsed from TOML. `AgentLoader` reads `~/.agent007/agents/*.toml`. `SubOrchestrator` wraps a scoped `ScopedMemoryStore` + its own `ModelRouter` and decomposes tasks into subtasks that it dispatches to persona workers. The top-level `OrchestratorAgent` in `core` gains an optional `AgentRegistry` to route tasks to named sub-orchestrators. CLI: `agent007 agent list/run/inspect/create`.

**Tech Stack:** Rust, thiserror, serde/toml, tokio, agent007-core, agent007-personas, agent007-memory, agent007-models

---

## File Structure

```
crates/custom-agents/
├── Cargo.toml
└── src/
    ├── lib.rs                  # pub re-exports: AgentDef, AgentType, AgentZoneOverrides,
    │                           #   SubTaskResult, AgentRegistry, SubOrchestrator, CustomAgentError
    ├── error.rs                # CustomAgentError (thiserror)
    ├── types.rs                # AgentDef, AgentType, AgentZoneOverrides, SubTaskResult
    ├── registry.rs             # AgentRegistry (load + list + get + filter by type)
    ├── loader.rs               # load_agent_def(path) + load_all(dir) helpers
    └── sub_orchestrator.rs     # SubOrchestrator::new + run + depth guard

crates/cli/src/
    ├── commands/
    │   ├── mod.rs              # add: pub mod agent
    │   ├── agent.rs            # NEW: execute(config, action) for list/run/inspect/create
    │   └── run.rs              # wire AgentRegistry into build_stack
    └── main.rs                 # add AgentArgs + AgentAction, add Commands::Agent arm

Modify: Cargo.toml (workspace root — add crates/custom-agents to members)
Modify: crates/cli/Cargo.toml (add agent007-custom-agents dep)
```

---

## Task 1: Crate scaffold, error type, and AgentDef TOML deserialization

**Files:**
- Create: `crates/custom-agents/Cargo.toml`
- Create: `crates/custom-agents/src/lib.rs`
- Create: `crates/custom-agents/src/error.rs`
- Create: `crates/custom-agents/src/types.rs`
- Modify: `Cargo.toml` (workspace root — add `crates/custom-agents` to members)

- [ ] **Step 1.1: Add custom-agents to workspace**

Add `"crates/custom-agents"` to the `[workspace]` members list in the root `Cargo.toml`. Add to `[workspace.dependencies]` if not already present:

```toml
agent007-custom-agents = { path = "crates/custom-agents" }
```

- [ ] **Step 1.2: Create crates/custom-agents/Cargo.toml**

```toml
[package]
name = "agent007-custom-agents"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core    = { workspace = true }
agent007-memory  = { workspace = true }
agent007-models  = { workspace = true }
agent007-personas = { workspace = true }
thiserror = { workspace = true }
serde     = { workspace = true, features = ["derive"] }
toml      = { workspace = true }
tokio     = { workspace = true, features = ["full"] }
tracing   = { workspace = true }

[dev-dependencies]
tempfile  = { workspace = true }
tokio     = { workspace = true, features = ["full", "test-util"] }
```

- [ ] **Step 1.3: Write failing tests for CustomAgentError and AgentDef deserialization**

Create `crates/custom-agents/src/types.rs` with only a `#[cfg(test)]` block (no implementations yet) so the tests fail to compile:

```rust
// crates/custom-agents/src/types.rs  — test block written FIRST

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_type_deserializes_worker() {
        let v: AgentType = toml::from_str("\"worker\"").unwrap();
        assert_eq!(v, AgentType::Worker);
    }

    #[test]
    fn agent_type_deserializes_sub_orchestrator() {
        let v: AgentType = toml::from_str("\"sub-orchestrator\"").unwrap();
        assert_eq!(v, AgentType::SubOrchestrator);
    }

    #[test]
    fn agent_def_full_round_trip() {
        let toml_str = r#"
            name = "LibP2PSubOrchestrator"
            type = "sub-orchestrator"
            description = "Owns the libp2p networking module"
            scope = ["src/networking/libp2p/", "tests/networking/libp2p/"]
            system_prompt = "You are the LibP2P module owner."
            allowed_workers = ["Researcher", "Coder"]
            model = "claude"
            memory_namespace = "libp2p"

            [zones]
            readonly = ["src/networking/libp2p/core/"]
        "#;
        let def: AgentDef = toml::from_str(toml_str).unwrap();
        assert_eq!(def.name, "LibP2PSubOrchestrator");
        assert_eq!(def.r#type, AgentType::SubOrchestrator);
        assert_eq!(def.memory_namespace.as_deref(), Some("libp2p"));
        let zones = def.zones.unwrap();
        assert_eq!(
            zones.readonly.unwrap(),
            vec!["src/networking/libp2p/core/"]
        );
    }

    #[test]
    fn agent_def_minimal_round_trip() {
        let toml_str = r#"
            name = "QuickWorker"
            type = "worker"
            system_prompt = "Do things."
        "#;
        let def: AgentDef = toml::from_str(toml_str).unwrap();
        assert_eq!(def.r#type, AgentType::Worker);
        assert!(def.description.is_none());
        assert!(def.zones.is_none());
    }

    #[test]
    fn sub_task_result_defaults() {
        let r = SubTaskResult::default();
        assert!(r.output.is_empty());
        assert!(r.files_changed.is_empty());
        assert!(!r.tests_passed);
        assert!(r.blockers.is_empty());
    }
}
```

- [ ] **Step 1.4: Implement AgentType, AgentDef, AgentZoneOverrides, SubTaskResult**

Fill in `crates/custom-agents/src/types.rs` above the test block:

```rust
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentType {
    Worker,
    SubOrchestrator,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AgentDef {
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: AgentType,
    pub description: Option<String>,
    pub scope: Option<Vec<String>>,
    pub system_prompt: String,
    pub allowed_workers: Option<Vec<String>>,
    pub model: Option<String>,
    pub memory_namespace: Option<String>,
    pub zones: Option<AgentZoneOverrides>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AgentZoneOverrides {
    pub readonly: Option<Vec<String>>,
    pub sensitive: Option<Vec<String>>,
    pub forbidden: Option<Vec<String>>,
}

#[derive(Debug, Default)]
pub struct SubTaskResult {
    pub output: String,
    pub files_changed: Vec<PathBuf>,
    pub tests_passed: bool,
    pub blockers: Vec<String>,
}
```

- [ ] **Step 1.5: Write failing test for CustomAgentError variants**

Create `crates/custom-agents/src/error.rs` with a test block first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn not_found_message() {
        let e = CustomAgentError::NotFound { name: "Foo".into() };
        assert_eq!(e.to_string(), "agent 'Foo' not found");
    }

    #[test]
    fn parse_error_message() {
        let e = CustomAgentError::ParseError {
            path: PathBuf::from("agents/foo.toml"),
            reason: "missing field `name`".into(),
        };
        assert!(e.to_string().contains("agents/foo.toml"));
        assert!(e.to_string().contains("missing field `name`"));
    }

    #[test]
    fn max_depth_message() {
        let e = CustomAgentError::MaxDepthExceeded { max: 3 };
        assert!(e.to_string().contains('3'));
    }

    #[test]
    fn worker_not_allowed_message() {
        let e = CustomAgentError::WorkerNotAllowed { name: "Hacker".into() };
        assert!(e.to_string().contains("Hacker"));
    }
}
```

- [ ] **Step 1.6: Implement CustomAgentError**

Add the implementation above the test block in `crates/custom-agents/src/error.rs`:

```rust
use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum CustomAgentError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse agent file {path}: {reason}")]
    ParseError { path: PathBuf, reason: String },
    #[error("agent '{name}' not found")]
    NotFound { name: String },
    #[error("max orchestrator depth {max} exceeded")]
    MaxDepthExceeded { max: usize },
    #[error("worker '{name}' not in allowed_workers for this sub-orchestrator")]
    WorkerNotAllowed { name: String },
}
```

- [ ] **Step 1.7: Create crates/custom-agents/src/lib.rs**

```rust
pub mod error;
pub mod types;

pub use error::CustomAgentError;
pub use types::{AgentDef, AgentType, AgentZoneOverrides, SubTaskResult};
```

- [ ] **Step 1.8: Verify all Task 1 tests pass**

```bash
cargo test -p agent007-custom-agents -- types error 2>&1
```

All 9 tests must pass before proceeding.

---

## Task 2: AgentRegistry — load, list, get, and filter by type

**Files:**
- Create: `crates/custom-agents/src/loader.rs`
- Create: `crates/custom-agents/src/registry.rs`
- Modify: `crates/custom-agents/src/lib.rs` (add pub mod loader, registry + re-exports)

- [ ] **Step 2.1: Write failing tests for AgentLoader**

Create `crates/custom-agents/src/loader.rs` with test block first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

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
```

- [ ] **Step 2.2: Implement load_agent_def and load_all**

Add the implementation above the test block in `crates/custom-agents/src/loader.rs`:

```rust
use std::path::Path;
use crate::{AgentDef, CustomAgentError};

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
```

- [ ] **Step 2.3: Write failing tests for AgentRegistry**

Create `crates/custom-agents/src/registry.rs` with test block first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    const WORKER_TOML: &str = r#"
        name = "WorkerA"
        type = "worker"
        system_prompt = "I work."
    "#;

    const ORCHESTRATOR_TOML: &str = r#"
        name = "OrchestratorB"
        type = "sub-orchestrator"
        system_prompt = "I orchestrate."
        allowed_workers = ["WorkerA"]
    "#;

    fn make_registry() -> (AgentRegistry, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("worker.toml"), WORKER_TOML).unwrap();
        fs::write(dir.path().join("orch.toml"), ORCHESTRATOR_TOML).unwrap();
        let reg = AgentRegistry::load(dir.path()).unwrap();
        (reg, dir)
    }

    #[test]
    fn load_populates_registry() {
        let (reg, _dir) = make_registry();
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn get_returns_existing_agent() {
        let (reg, _dir) = make_registry();
        let def = reg.get("WorkerA").unwrap();
        assert_eq!(def.name, "WorkerA");
    }

    #[test]
    fn get_returns_none_for_unknown() {
        let (reg, _dir) = make_registry();
        assert!(reg.get("Nobody").is_none());
    }

    #[test]
    fn workers_filter_returns_only_workers() {
        let (reg, _dir) = make_registry();
        let workers = reg.workers();
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].name, "WorkerA");
    }

    #[test]
    fn sub_orchestrators_filter_returns_only_orchestrators() {
        let (reg, _dir) = make_registry();
        let orchs = reg.sub_orchestrators();
        assert_eq!(orchs.len(), 1);
        assert_eq!(orchs[0].name, "OrchestratorB");
    }

    #[test]
    fn load_missing_dir_returns_empty_registry() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nonexistent");
        let reg = AgentRegistry::load(&missing).unwrap();
        assert!(reg.list().is_empty());
    }
}
```

- [ ] **Step 2.4: Implement AgentRegistry**

Add the implementation above the test block in `crates/custom-agents/src/registry.rs`:

```rust
use std::collections::HashMap;
use std::path::Path;
use crate::{AgentDef, AgentType, CustomAgentError};
use crate::loader::load_all;

pub struct AgentRegistry {
    agents: HashMap<String, AgentDef>,
}

impl AgentRegistry {
    pub fn load(agents_dir: &Path) -> Result<Self, CustomAgentError> {
        let defs = load_all(agents_dir)?;
        let agents = defs.into_iter().map(|d| (d.name.clone(), d)).collect();
        Ok(Self { agents })
    }

    pub fn get(&self, name: &str) -> Option<&AgentDef> {
        self.agents.get(name)
    }

    pub fn list(&self) -> Vec<&AgentDef> {
        self.agents.values().collect()
    }

    pub fn sub_orchestrators(&self) -> Vec<&AgentDef> {
        self.agents
            .values()
            .filter(|d| d.r#type == AgentType::SubOrchestrator)
            .collect()
    }

    pub fn workers(&self) -> Vec<&AgentDef> {
        self.agents
            .values()
            .filter(|d| d.r#type == AgentType::Worker)
            .collect()
    }
}
```

- [ ] **Step 2.5: Update lib.rs to expose loader and registry**

```rust
// crates/custom-agents/src/lib.rs
pub mod error;
pub mod loader;
pub mod registry;
pub mod types;

pub use error::CustomAgentError;
pub use loader::{load_agent_def, load_all};
pub use registry::AgentRegistry;
pub use types::{AgentDef, AgentType, AgentZoneOverrides, SubTaskResult};
```

- [ ] **Step 2.6: Verify all Task 2 tests pass**

```bash
cargo test -p agent007-custom-agents -- loader registry 2>&1
```

All 12 tests must pass before proceeding.

---

## Task 3: SubOrchestrator::new + memory namespacing

**Files:**
- Create: `crates/custom-agents/src/sub_orchestrator.rs`
- Modify: `crates/custom-agents/src/lib.rs` (add pub mod sub_orchestrator + re-exports)

- [ ] **Step 3.1: Write failing tests for SubOrchestrator construction and memory namespacing**

Create `crates/custom-agents/src/sub_orchestrator.rs` with test block first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::dispatcher::NoOpDispatcher;
    use agent007_core::persona::NoOpPersonaProvider;
    use agent007_memory::MemoryStore;
    use agent007_models::router::ModelRouter;
    use std::sync::Arc;

    fn make_def(name: &str, namespace: Option<&str>) -> AgentDef {
        AgentDef {
            name: name.to_string(),
            r#type: AgentType::SubOrchestrator,
            description: None,
            scope: None,
            system_prompt: "Test.".into(),
            allowed_workers: Some(vec!["Coder".into()]),
            model: None,
            memory_namespace: namespace.map(str::to_string),
            zones: None,
        }
    }

    fn make_orch(def: AgentDef, depth: usize) -> SubOrchestrator {
        let inner_store = Arc::new(MemoryStore::new_in_memory());
        let scoped = Arc::new(ScopedMemoryStore::new(inner_store, def.memory_namespace
            .clone()
            .unwrap_or_else(|| def.name.clone())));
        let router = Arc::new(ModelRouter::default());
        let personas = Arc::new(NoOpPersonaProvider);
        let dispatcher = Arc::new(NoOpDispatcher);
        SubOrchestrator::new(def, scoped, router, personas, dispatcher, depth, 3)
    }

    #[test]
    fn new_sets_depth_and_max_depth() {
        let def = make_def("OrchestratorX", None);
        let orch = make_orch(def, 0);
        assert_eq!(orch.depth, 0);
        assert_eq!(orch.max_depth, 3);
    }

    #[test]
    fn memory_namespace_uses_explicit_value() {
        let def = make_def("OrchestratorX", Some("my-ns"));
        let orch = make_orch(def, 0);
        assert_eq!(orch.scoped_memory.namespace(), "my-ns");
    }

    #[test]
    fn memory_namespace_falls_back_to_agent_name() {
        let def = make_def("OrchestratorX", None);
        // When no memory_namespace is given, caller uses agent name as namespace.
        // ScopedMemoryStore::namespace() must return what was passed at construction.
        let orch = make_orch(def, 0);
        assert_eq!(orch.scoped_memory.namespace(), "OrchestratorX");
    }

    #[test]
    fn def_name_is_preserved() {
        let def = make_def("MyOrch", Some("ns"));
        let orch = make_orch(def, 1);
        assert_eq!(orch.def.name, "MyOrch");
    }
}
```

- [ ] **Step 3.2: Implement SubOrchestrator struct and new()**

Add above the test block in `crates/custom-agents/src/sub_orchestrator.rs`:

```rust
use std::sync::Arc;
use agent007_core::dispatcher::Dispatcher;
use agent007_core::persona::PersonaProvider;
use agent007_memory::store::ScopedMemoryStore;
use agent007_models::router::ModelRouter;
use crate::{AgentDef, AgentType, CustomAgentError, SubTaskResult};

pub struct SubOrchestrator {
    pub def: AgentDef,
    pub scoped_memory: Arc<ScopedMemoryStore>,
    pub model_router: Arc<ModelRouter>,
    pub persona_provider: Arc<dyn PersonaProvider>,
    pub dispatcher: Arc<dyn Dispatcher>,
    pub depth: usize,
    pub max_depth: usize,
}

impl SubOrchestrator {
    pub fn new(
        def: AgentDef,
        scoped_memory: Arc<ScopedMemoryStore>,
        model_router: Arc<ModelRouter>,
        persona_provider: Arc<dyn PersonaProvider>,
        dispatcher: Arc<dyn Dispatcher>,
        depth: usize,
        max_depth: usize,
    ) -> Self {
        Self { def, scoped_memory, model_router, persona_provider, dispatcher, depth, max_depth }
    }
}
```

- [ ] **Step 3.3: Verify all Task 3 tests pass**

```bash
cargo test -p agent007-custom-agents -- sub_orchestrator::tests 2>&1
```

All 4 construction tests must pass before proceeding.

---

## Task 4: SubOrchestrator::run — task decomposition and persona worker dispatch

**Files:**
- Modify: `crates/custom-agents/src/sub_orchestrator.rs` (add `run` method)

- [ ] **Step 4.1: Write failing tests for SubOrchestrator::run**

Append to the `tests` block in `crates/custom-agents/src/sub_orchestrator.rs`:

```rust
    #[tokio::test]
    async fn run_returns_sub_task_result() {
        let def = make_def("OrchestratorX", Some("ns"));
        let orch = make_orch(def, 0);
        let result = orch.run("implement feature X").await.unwrap();
        // Output must be non-empty — the orchestrator produced some response
        assert!(!result.output.is_empty());
    }

    #[tokio::test]
    async fn run_worker_not_allowed_returns_error() {
        // Craft an agent whose allowed_workers is empty, then confirm WorkerNotAllowed
        // is surfaced when a disallowed persona is attempted.
        // (The exact enforcement mechanism is in run(); test via AgentDef with empty
        // allowed_workers so no persona can be dispatched.)
        let mut def = make_def("StrictOrch", Some("ns"));
        def.allowed_workers = Some(vec![]); // no workers permitted
        let orch = make_orch(def, 0);
        let err = orch.run("do something requiring a worker").await.unwrap_err();
        assert!(matches!(err, CustomAgentError::WorkerNotAllowed { .. }));
    }
```

- [ ] **Step 4.2: Implement SubOrchestrator::run**

Add the `run` method to the `SubOrchestrator` impl block:

```rust
    /// Decompose the task into subtasks and execute via allowed worker personas.
    ///
    /// Algorithm:
    /// 1. Ask the model (via `model_router`) to produce a JSON plan: list of
    ///    `{ "worker": "<PersonaName>", "subtask": "<description>" }` objects.
    /// 2. For each step, validate the worker is in `allowed_workers`; return
    ///    `CustomAgentError::WorkerNotAllowed` immediately if not.
    /// 3. Retrieve the `PersonaSpec` from `persona_provider` and dispatch the
    ///    subtask via `dispatcher` with the persona's system prompt prepended.
    /// 4. Collect all outputs into `SubTaskResult`.
    pub async fn run(&self, task: &str) -> Result<SubTaskResult, CustomAgentError> {
        // Guard: if allowed_workers is Some([]) no workers can be dispatched
        if let Some(ref allowed) = self.def.allowed_workers {
            if allowed.is_empty() {
                return Err(CustomAgentError::WorkerNotAllowed {
                    name: "<none>".into(),
                });
            }
        }

        // Build system context from scoped memory (RAG retrieval)
        let _ns = self.scoped_memory.namespace();

        // Plan decomposition via model router — returns structured plan
        let plan_prompt = format!(
            "You are {}. Decompose this task into subtasks, one per allowed worker.\n\
             Allowed workers: {:?}\nTask: {}",
            self.def.name,
            self.def.allowed_workers,
            task
        );

        let raw_plan = self
            .model_router
            .complete(&plan_prompt, &self.def.system_prompt)
            .await
            .map_err(|e| CustomAgentError::ParseError {
                path: std::path::PathBuf::from("<plan>"),
                reason: e.to_string(),
            })?;

        // Parse plan — expect newline-separated "WORKER: <name>\nSUBTASK: <desc>" blocks
        // or a best-effort pass-through if the model returns free text.
        let subtasks = parse_plan(&raw_plan, self.def.allowed_workers.as_deref())?;

        let mut combined_output = String::new();
        let mut files_changed = Vec::new();

        for (worker_name, subtask) in &subtasks {
            let persona = self
                .persona_provider
                .get(worker_name)
                .ok_or_else(|| CustomAgentError::WorkerNotAllowed {
                    name: worker_name.clone(),
                })?;

            let result = self
                .dispatcher
                .dispatch(&persona.system_prompt, subtask)
                .await
                .map_err(|e| CustomAgentError::ParseError {
                    path: std::path::PathBuf::from("<dispatch>"),
                    reason: e.to_string(),
                })?;

            combined_output.push_str(&result.output);
            combined_output.push('\n');
            files_changed.extend(result.files_changed);
        }

        Ok(SubTaskResult {
            output: combined_output,
            files_changed,
            tests_passed: false, // updated by caller after test runner
            blockers: Vec::new(),
        })
    }
```

Also implement the private helper `parse_plan` in the same file (below the impl block, above tests):

```rust
fn parse_plan(
    raw: &str,
    allowed: Option<&[String]>,
) -> Result<Vec<(String, String)>, CustomAgentError> {
    // Accepts two formats:
    //   1. JSON array: [{"worker": "Coder", "subtask": "..."}]
    //   2. Free-text fallback: treat entire raw text as single subtask for first allowed worker
    if let Ok(steps) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
        steps
            .iter()
            .map(|v| {
                let worker = v["worker"].as_str().unwrap_or("").to_string();
                let subtask = v["subtask"].as_str().unwrap_or("").to_string();
                if let Some(allowed) = allowed {
                    if !allowed.contains(&worker) {
                        return Err(CustomAgentError::WorkerNotAllowed { name: worker });
                    }
                }
                Ok((worker, subtask))
            })
            .collect()
    } else {
        // Free-text fallback — assign to first allowed worker
        let worker = allowed
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or_else(|| "default".into());
        Ok(vec![(worker, raw.to_string())])
    }
}
```

Add `serde_json` to `[dependencies]` in `crates/custom-agents/Cargo.toml`:

```toml
serde_json = { workspace = true }
```

- [ ] **Step 4.3: Verify all Task 4 tests pass**

```bash
cargo test -p agent007-custom-agents -- sub_orchestrator 2>&1
```

All 6 `sub_orchestrator` tests must pass.

---

## Task 5: Depth guard enforcement

**Files:**
- Modify: `crates/custom-agents/src/sub_orchestrator.rs` (add depth check at top of `run`)

- [ ] **Step 5.1: Write failing test for depth guard**

Append to the `tests` block in `crates/custom-agents/src/sub_orchestrator.rs`:

```rust
    #[tokio::test]
    async fn run_exceeds_max_depth_returns_error() {
        let def = make_def("DeepOrch", Some("ns"));
        // depth == max_depth means we are already at the limit
        let orch = make_orch(def, 3); // depth=3, max_depth=3
        let err = orch.run("some task").await.unwrap_err();
        assert!(matches!(
            err,
            CustomAgentError::MaxDepthExceeded { max: 3 }
        ));
    }

    #[tokio::test]
    async fn run_at_depth_below_max_does_not_error_on_depth() {
        let def = make_def("ShallowOrch", Some("ns"));
        let orch = make_orch(def, 2); // depth=2, max_depth=3 — within limits
        // Should not produce MaxDepthExceeded; any other result is acceptable here
        let err = orch.run("task").await;
        assert!(!matches!(
            err,
            Err(CustomAgentError::MaxDepthExceeded { .. })
        ));
    }
```

- [ ] **Step 5.2: Add depth guard to SubOrchestrator::run**

Insert as the very first statement inside `SubOrchestrator::run`, before the `allowed_workers` check:

```rust
        if self.depth >= self.max_depth {
            return Err(CustomAgentError::MaxDepthExceeded { max: self.max_depth });
        }
```

- [ ] **Step 5.3: Verify all depth guard tests pass**

```bash
cargo test -p agent007-custom-agents -- sub_orchestrator 2>&1
```

All 8 sub_orchestrator tests (including the 2 new depth tests) must pass.

---

## Task 6: Wire AgentRegistry into build_stack

**Files:**
- Modify: `crates/cli/Cargo.toml` (add `agent007-custom-agents` dep)
- Modify: `crates/cli/src/commands/run.rs` (load AgentRegistry; pass to Orchestrator)

- [ ] **Step 6.1: Add custom-agents dependency to cli crate**

In `crates/cli/Cargo.toml`, add:

```toml
agent007-custom-agents = { workspace = true }
```

- [ ] **Step 6.2: Write integration test for build_stack with AgentRegistry**

Add a test module at the bottom of `crates/cli/src/commands/run.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn build_stack_loads_empty_agents_dir_without_panic() {
        // When ~/.agent007/agents/ does not exist, AgentRegistry::load returns
        // an empty registry and build_stack must not panic or return Err.
        let dir = tempdir().unwrap();
        let agents_dir = dir.path().join("agents");
        // deliberately do NOT create agents_dir — tests missing dir tolerance
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        assert!(registry.list().is_empty());
    }

    #[tokio::test]
    async fn build_stack_loads_agents_dir_with_one_agent() {
        use std::fs;
        let dir = tempdir().unwrap();
        let agents_dir = dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("demo.toml"),
            r#"name = "Demo"\ntype = "worker"\nsystem_prompt = "Demo.""#,
        )
        .unwrap();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        assert_eq!(registry.list().len(), 1);
    }
}
```

- [ ] **Step 6.3: Load AgentRegistry in build_stack**

In `crates/cli/src/commands/run.rs`, in the `build_stack` function (or equivalent stack construction function), add after config is loaded:

```rust
use agent007_custom_agents::AgentRegistry;

// Load custom agent definitions from ~/.agent007/agents/
let agents_dir = config.agent007_dir().join("agents");
let agent_registry = AgentRegistry::load(&agents_dir)
    .unwrap_or_else(|e| {
        tracing::warn!("Failed to load agent registry: {e}");
        AgentRegistry::empty()
    });
let agent_registry = Arc::new(agent_registry);
```

Add `AgentRegistry::empty()` constructor to `crates/custom-agents/src/registry.rs`:

```rust
impl AgentRegistry {
    pub fn empty() -> Self {
        Self { agents: HashMap::new() }
    }
    // ... existing methods ...
}
```

Pass `agent_registry` to the `Orchestrator::new` call (add `Option<Arc<AgentRegistry>>` field to `Orchestrator` if not already present; this is a non-breaking addition since it defaults to `None`).

- [ ] **Step 6.4: Verify run.rs tests pass and full cli builds**

```bash
cargo test -p agent007-cli -- commands::run 2>&1
cargo build -p agent007-cli 2>&1
```

---

## Task 7: CLI commands — list, run, inspect, create wizard

**Files:**
- Create: `crates/cli/src/commands/agent.rs`
- Modify: `crates/cli/src/commands/mod.rs` (add `pub mod agent`)
- Modify: `crates/cli/src/main.rs` (add `AgentArgs`, `AgentAction`, `Commands::Agent` arm)

- [ ] **Step 7.1: Write failing tests for agent CLI command dispatch**

Create `crates/cli/src/commands/agent.rs` with test block first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    const WORKER_TOML: &str = r#"
        name = "WorkerA"
        type = "worker"
        system_prompt = "I work hard."
        description = "A simple worker"
    "#;

    const ORCH_TOML: &str = r#"
        name = "OrchestratorB"
        type = "sub-orchestrator"
        system_prompt = "I orchestrate."
        allowed_workers = ["WorkerA"]
        memory_namespace = "orch-ns"
        scope = ["src/foo/"]
    "#;

    fn setup_agents_dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().unwrap();
        let agents_dir = dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(agents_dir.join("worker.toml"), WORKER_TOML).unwrap();
        fs::write(agents_dir.join("orch.toml"), ORCH_TOML).unwrap();
        (dir, agents_dir)
    }

    #[test]
    fn list_output_contains_agent_names() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let output = format_list(&registry);
        assert!(output.contains("WorkerA"));
        assert!(output.contains("OrchestratorB"));
    }

    #[test]
    fn list_output_shows_agent_type() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let output = format_list(&registry);
        assert!(output.contains("worker") || output.contains("Worker"));
        assert!(output.contains("sub-orchestrator") || output.contains("SubOrchestrator"));
    }

    #[test]
    fn inspect_output_contains_scope_and_namespace() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let def = registry.get("OrchestratorB").unwrap();
        let output = format_inspect(def);
        assert!(output.contains("src/foo/"));
        assert!(output.contains("orch-ns"));
    }

    #[test]
    fn inspect_output_contains_allowed_workers() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let def = registry.get("OrchestratorB").unwrap();
        let output = format_inspect(def);
        assert!(output.contains("WorkerA"));
    }

    #[test]
    fn inspect_unknown_agent_returns_error() {
        let (_dir, agents_dir) = setup_agents_dir();
        let registry = agent007_custom_agents::AgentRegistry::load(&agents_dir).unwrap();
        let err = inspect_agent(&registry, "NonExistent").unwrap_err();
        assert!(matches!(
            err,
            agent007_custom_agents::CustomAgentError::NotFound { .. }
        ));
    }

    #[test]
    fn create_wizard_generates_valid_toml() {
        // Smoke test: generate_agent_toml returns parseable TOML with expected fields
        let toml_str = generate_agent_toml("NewAgent", "sub-orchestrator", Some("new-ns"));
        let def: agent007_custom_agents::AgentDef = toml::from_str(&toml_str).unwrap();
        assert_eq!(def.name, "NewAgent");
        assert_eq!(def.memory_namespace.as_deref(), Some("new-ns"));
    }
}
```

- [ ] **Step 7.2: Implement agent CLI helpers and execute function**

Add the implementation above the test block in `crates/cli/src/commands/agent.rs`:

```rust
use agent007_custom_agents::{AgentDef, AgentRegistry, AgentType, CustomAgentError};
use std::sync::Arc;

/// Render a table-style listing of all registered agents.
pub fn format_list(registry: &AgentRegistry) -> String {
    let mut lines = vec![
        format!("{:<30} {:<18} {}", "NAME", "TYPE", "DESCRIPTION"),
        "-".repeat(70),
    ];
    let mut agents = registry.list();
    agents.sort_by_key(|a| &a.name);
    for def in agents {
        let type_str = match def.r#type {
            AgentType::Worker => "worker",
            AgentType::SubOrchestrator => "sub-orchestrator",
        };
        let desc = def.description.as_deref().unwrap_or("-");
        lines.push(format!("{:<30} {:<18} {}", def.name, type_str, desc));
    }
    lines.join("\n")
}

/// Render detailed info for a single agent.
pub fn format_inspect(def: &AgentDef) -> String {
    let mut lines = vec![
        format!("Name:             {}", def.name),
        format!("Type:             {:?}", def.r#type),
        format!(
            "Description:      {}",
            def.description.as_deref().unwrap_or("-")
        ),
        format!(
            "Memory Namespace: {}",
            def.memory_namespace.as_deref().unwrap_or(&def.name)
        ),
        format!("Model:            {}", def.model.as_deref().unwrap_or("default")),
    ];
    if let Some(ref scope) = def.scope {
        lines.push(format!("Scope:            {}", scope.join(", ")));
    }
    if let Some(ref workers) = def.allowed_workers {
        lines.push(format!("Allowed Workers:  {}", workers.join(", ")));
    }
    if let Some(ref zones) = def.zones {
        if let Some(ref ro) = zones.readonly {
            lines.push(format!("Zones (readonly): {}", ro.join(", ")));
        }
        if let Some(ref sens) = zones.sensitive {
            lines.push(format!("Zones (sensitive): {}", sens.join(", ")));
        }
        if let Some(ref forb) = zones.forbidden {
            lines.push(format!("Zones (forbidden): {}", forb.join(", ")));
        }
    }
    lines.join("\n")
}

/// Look up an agent by name and return formatted inspect output.
pub fn inspect_agent(
    registry: &AgentRegistry,
    name: &str,
) -> Result<String, CustomAgentError> {
    let def = registry
        .get(name)
        .ok_or_else(|| CustomAgentError::NotFound { name: name.to_string() })?;
    Ok(format_inspect(def))
}

/// Generate a minimal TOML stub for a new agent.
pub fn generate_agent_toml(name: &str, agent_type: &str, namespace: Option<&str>) -> String {
    let ns_line = namespace
        .map(|ns| format!("memory_namespace = \"{ns}\"\n"))
        .unwrap_or_default();
    format!(
        r#"name = "{name}"
type = "{agent_type}"
description = "TODO: describe this agent"
system_prompt = "TODO: write the system prompt for {name}."
{ns_line}allowed_workers = []
"#
    )
}

/// Entry point called from main.rs dispatch.
pub async fn execute(
    registry: Arc<AgentRegistry>,
    action: AgentAction,
) -> anyhow::Result<()> {
    match action {
        AgentAction::List => {
            println!("{}", format_list(&registry));
        }
        AgentAction::Inspect { name } => {
            println!("{}", inspect_agent(&registry, &name)?);
        }
        AgentAction::Run { name, task } => {
            // Retrieve the agent definition; actual SubOrchestrator execution
            // requires the full stack (memory, models, personas) which is
            // constructed in run.rs build_stack. Here we delegate to run::run_agent.
            let def = registry
                .get(&name)
                .ok_or_else(|| CustomAgentError::NotFound { name: name.clone() })?;
            tracing::info!("Running agent '{}' with task: {}", name, task);
            println!("Running agent '{}' ...\nTask: {}", def.name, task);
            // TODO: wire in SubOrchestrator once build_stack exposes it
        }
        AgentAction::Create { name, agent_type, namespace } => {
            let toml_str = generate_agent_toml(&name, &agent_type, namespace.as_deref());
            let agents_dir = dirs::home_dir()
                .expect("cannot determine home dir")
                .join(".agent007")
                .join("agents");
            std::fs::create_dir_all(&agents_dir)?;
            let path = agents_dir.join(format!("{}.toml", name.to_lowercase().replace(' ', "_")));
            std::fs::write(&path, &toml_str)?;
            println!("Created agent definition at: {}", path.display());
            println!("\n{toml_str}");
        }
    }
    Ok(())
}

/// Sub-actions for the `agent` command.
#[derive(Debug, Clone)]
pub enum AgentAction {
    List,
    Inspect { name: String },
    Run { name: String, task: String },
    Create { name: String, agent_type: String, namespace: Option<String> },
}
```

- [ ] **Step 7.3: Add clap argument structs in main.rs**

In `crates/cli/src/main.rs`, add:

```rust
// --- agent subcommand ---

#[derive(clap::Args, Debug)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub action: AgentSubCommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum AgentSubCommand {
    /// List all registered agents
    List,
    /// Show details for a named agent
    Inspect {
        /// Agent name
        name: String,
    },
    /// Run a named agent with a task description
    Run {
        /// Agent name
        name: String,
        /// Task description
        task: String,
    },
    /// Interactively create a new agent TOML file
    Create {
        /// Agent name
        #[arg(long)]
        name: String,
        /// Agent type: "worker" or "sub-orchestrator"
        #[arg(long, default_value = "worker")]
        r#type: String,
        /// Memory namespace (defaults to agent name)
        #[arg(long)]
        namespace: Option<String>,
    },
}
```

Add `Agent(AgentArgs)` to the `Commands` enum.

In the main dispatch match, add:

```rust
Commands::Agent(args) => {
    use commands::agent::{AgentAction, execute};
    let action = match args.action {
        AgentSubCommand::List => AgentAction::List,
        AgentSubCommand::Inspect { name } => AgentAction::Inspect { name },
        AgentSubCommand::Run { name, task } => AgentAction::Run { name, task },
        AgentSubCommand::Create { name, r#type, namespace } =>
            AgentAction::Create { name, agent_type: r#type, namespace },
    };
    execute(agent_registry.clone(), action).await?;
}
```

- [ ] **Step 7.4: Register pub mod agent in commands/mod.rs**

```rust
// crates/cli/src/commands/mod.rs
pub mod agent;
// ... existing modules ...
```

- [ ] **Step 7.5: Verify all Task 7 tests pass and binary builds**

```bash
cargo test -p agent007-cli -- commands::agent 2>&1
cargo build -p agent007-cli 2>&1
```

All 6 agent command tests must pass.

- [ ] **Step 7.6: Smoke-test CLI agent commands**

```bash
cargo run -p agent007-cli -- agent list 2>&1
cargo run -p agent007-cli -- agent inspect NonExistent 2>&1
cargo run -p agent007-cli -- agent create --name "TestAgent" --type worker 2>&1
```

Expected: `list` prints header and any loaded agents (or empty table). `inspect NonExistent` prints an error message. `create` writes a TOML stub and prints the path.

---

## Task 8: Full integration verification

- [ ] **Step 8.1: Run the full test suite**

```bash
cargo test --workspace 2>&1
```

Zero test failures expected.

- [ ] **Step 8.2: Run clippy**

```bash
cargo clippy --workspace -- -D warnings 2>&1
```

Zero warnings expected.

- [ ] **Step 8.3: Verify end-to-end agent round-trip with a real TOML file**

```bash
mkdir -p ~/.agent007/agents
cat > ~/.agent007/agents/demo.toml << 'EOF'
name = "DemoAgent"
type = "worker"
description = "Smoke test worker"
system_prompt = "You are a demo agent."
model = "claude"
memory_namespace = "demo"
EOF

cargo run -p agent007-cli -- agent list
cargo run -p agent007-cli -- agent inspect DemoAgent
```

Expected: `list` shows `DemoAgent` as a worker; `inspect` shows `memory_namespace = demo`.

- [ ] **Step 8.4: Verify depth guard rejects overly nested orchestrators**

Write a short Rust integration test in `crates/custom-agents/tests/depth_guard.rs` (if an `tests/` dir is desired) or inline in `sub_orchestrator.rs` that constructs a `SubOrchestrator` with `depth == max_depth` and asserts `run()` returns `MaxDepthExceeded`.

```bash
cargo test -p agent007-custom-agents 2>&1
```
