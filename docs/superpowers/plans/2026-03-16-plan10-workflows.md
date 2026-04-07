# Workflows Crate Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `agent007-workflows` crate providing TOML-defined multi-agent pipelines (DAGs) with typed inputs/outputs, model overrides, human-in-the-loop approval gates, token/cost budget guard, and `agent007 workflow` CLI commands.

**Architecture:** New `crates/workflows` crate. `WorkflowDef` parsed from TOML. `WorkflowRunner` resolves the DAG (topological sort), executes steps in dependency order (concurrent where possible), handles `requires_approval` gates by pausing and prompting via stdin, and enforces budget limits. Depends on `agent007-core`, `agent007-personas`, `agent007-models`.

**Tech Stack:** Rust, thiserror, serde/toml, tokio, petgraph = "0.6" (DAG topology), agent007-core, agent007-personas

---

## File Structure

```
crates/workflows/
├── Cargo.toml
└── src/
    ├── lib.rs          # pub re-exports: WorkflowDef, WorkflowRunner, WorkflowError, WorkflowResult
    ├── error.rs        # WorkflowError (thiserror)
    ├── types.rs        # WorkflowDef, StepDef, BudgetConfig, WorkflowResult, BudgetUsed
    ├── dag.rs          # DAG validation + topological batching (petgraph)
    ├── runner.rs       # WorkflowRunner + budget tracking
    ├── approval.rs     # human-in-the-loop gate (reads stdin, opens $EDITOR)
    └── loader.rs       # load WorkflowDef from TOML file / directory

crates/cli/src/commands/
└── workflow.rs         # CLI subcommand: run, list, validate, show

Modified files:
    Cargo.toml                              (root workspace — add crates/workflows + petgraph dep)
    crates/cli/Cargo.toml                   (add agent007-workflows dep)
    crates/cli/src/main.rs                  (add Commands::Workflow arm + WorkflowArgs)
    crates/cli/src/commands/mod.rs          (add: pub mod workflow)
    crates/cli/src/commands/run.rs          (add workflow_runner field to Stack; wire in build_stack)
```

**Prerequisites:** Plans 1–5 complete (core, models, personas crates built and tested).

---

## Chunk 1: Crate Scaffold + Error Type + Types

### Task 1: Add workflows crate to workspace; define error type and all public types

**Files:**
- Create: `crates/workflows/Cargo.toml`
- Create: `crates/workflows/src/lib.rs`
- Create: `crates/workflows/src/error.rs`
- Create: `crates/workflows/src/types.rs`
- Modify: `Cargo.toml` (root — add `"crates/workflows"` to members; add `petgraph = "0.6"` to `[workspace.dependencies]`)

- [ ] **Step 1: Add workflows to workspace and add petgraph workspace dep**

In root `Cargo.toml`, add `"crates/workflows"` to the `members` array and add to `[workspace.dependencies]`:

```toml
petgraph = "0.6"
```

- [ ] **Step 2: Create `crates/workflows/Cargo.toml`**

```toml
[package]
name = "agent007-workflows"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core     = { path = "../core" }
agent007-models   = { path = "../models" }
agent007-personas = { path = "../personas" }
thiserror         = { workspace = true }
serde             = { workspace = true }
serde_json        = { workspace = true }
toml              = { workspace = true }
tokio             = { workspace = true }
futures           = { workspace = true }
petgraph          = { workspace = true }
tera              = { workspace = true }
tracing           = { workspace = true }
async-trait       = { workspace = true }

[dev-dependencies]
tempfile  = { workspace = true }
tokio     = { workspace = true }
```

- [ ] **Step 3: Create `crates/workflows/src/error.rs`**

```rust
use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum WorkflowError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse workflow {path}: {reason}")]
    ParseError { path: PathBuf, reason: String },

    #[error("step '{id}' references unknown input '{input}'")]
    UnknownInput { id: String, input: String },

    #[error("workflow has a dependency cycle")]
    CycleDetected,

    #[error("step '{id}' failed: {reason}")]
    StepFailed { id: String, reason: String },

    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("approval denied for step '{0}'")]
    ApprovalDenied(String),

    #[error("template render error for step '{id}': {reason}")]
    TemplateError { id: String, reason: String },

    #[error("persona '{0}' not found")]
    PersonaNotFound(String),
}
```

- [ ] **Step 4: Create `crates/workflows/src/types.rs`**

```rust
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Debug, Clone)]
pub struct WorkflowDef {
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<StepDef>,
    pub budget: Option<BudgetConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct StepDef {
    pub id: String,
    pub agent: String,
    pub model: Option<String>,
    pub inputs: Option<Vec<String>>,
    pub depends_on: Option<Vec<String>>,
    pub prompt: String,
    pub output: Option<String>,
    pub requires_approval: Option<bool>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct BudgetConfig {
    pub max_tokens_per_session: Option<u64>,
    pub max_usd_per_task: Option<f64>,
    pub alert_at_percent: Option<u8>,
    pub on_exceed: Option<String>,  // "pause" | "stop" | "alert-only"
}

#[derive(Debug, Default)]
pub struct WorkflowResult {
    pub outputs: HashMap<String, String>,
    pub steps_completed: usize,
    pub steps_total: usize,
    pub budget_used: BudgetUsed,
}

#[derive(Debug, Default, Clone)]
pub struct BudgetUsed {
    pub tokens: u64,
    pub estimated_usd: f64,
}
```

- [ ] **Step 5: Create `crates/workflows/src/lib.rs`**

```rust
pub mod error;
pub mod types;
pub mod dag;
pub mod runner;
pub mod approval;
pub mod loader;

pub use error::WorkflowError;
pub use types::{WorkflowDef, StepDef, BudgetConfig, WorkflowResult, BudgetUsed};
pub use runner::WorkflowRunner;
pub use loader::WorkflowLoader;
```

- [ ] **Step 6: Write failing tests for types (TOML round-trip deserialization)**

In `crates/workflows/src/types.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_TOML: &str = r#"
name = "Test Workflow"

[[steps]]
id = "step1"
agent = "Researcher"
prompt = "Research {{task}}"
output = "notes"
"#;

    const FULL_TOML: &str = r#"
name = "TDD Feature Development"
description = "Research → Architect → Coder"

[[steps]]
id = "research"
agent = "Researcher"
model = "claude"
prompt = "Research best practices for: {{task}}"
output = "research_notes"

[[steps]]
id = "architect"
agent = "Architect"
model = "claude"
inputs = ["research_notes"]
prompt = "Design an implementation plan"
output = "plan"
requires_approval = true

[[steps]]
id = "implement"
agent = "Coder"
model = "codex"
inputs = ["plan"]
depends_on = ["architect"]
prompt = "Implement until all tests pass"
output = "implementation"

[budget]
max_tokens_per_session = 500000
max_usd_per_task = 2.00
alert_at_percent = 80
on_exceed = "pause"
"#;

    #[test]
    fn deserialize_minimal_workflow() {
        let def: WorkflowDef = toml::from_str(MINIMAL_TOML).unwrap();
        assert_eq!(def.name, "Test Workflow");
        assert_eq!(def.steps.len(), 1);
        assert_eq!(def.steps[0].id, "step1");
        assert_eq!(def.steps[0].agent, "Researcher");
        assert!(def.budget.is_none());
    }

    #[test]
    fn deserialize_full_workflow() {
        let def: WorkflowDef = toml::from_str(FULL_TOML).unwrap();
        assert_eq!(def.name, "TDD Feature Development");
        assert_eq!(def.steps.len(), 3);
        let architect = &def.steps[1];
        assert_eq!(architect.id, "architect");
        assert_eq!(architect.requires_approval, Some(true));
        assert_eq!(architect.inputs.as_ref().unwrap(), &["research_notes"]);
        let budget = def.budget.as_ref().unwrap();
        assert_eq!(budget.max_tokens_per_session, Some(500_000));
        assert_eq!(budget.on_exceed.as_deref(), Some("pause"));
    }

    #[test]
    fn step_optional_fields_default_to_none() {
        let def: WorkflowDef = toml::from_str(MINIMAL_TOML).unwrap();
        let step = &def.steps[0];
        assert!(step.model.is_none());
        assert!(step.inputs.is_none());
        assert!(step.depends_on.is_none());
        assert!(step.requires_approval.is_none());
    }
}
```

- [ ] **Step 7: Run tests (expect compile failure until stub modules exist)**

```bash
cargo test -p agent007-workflows 2>&1 | head -40
```

Create stub files so the crate compiles:

`crates/workflows/src/dag.rs`:
```rust
// stub
```

`crates/workflows/src/runner.rs`:
```rust
// stub
```

`crates/workflows/src/approval.rs`:
```rust
// stub
```

`crates/workflows/src/loader.rs`:
```rust
// stub
```

- [ ] **Step 8: Run tests again (expect green)**

```bash
cargo test -p agent007-workflows -- types::tests
```

- [ ] **Step 9: Commit**

```
git add crates/workflows/ Cargo.toml
git commit -m "feat(workflows): scaffold crate with error type and TOML-deserialisable types"
```

---

## Chunk 2: DAG Validation + Topological Batching

### Task 2: Implement DAG validation and topological sort with cycle detection

**Files:**
- Implement: `crates/workflows/src/dag.rs`

The DAG module uses `petgraph::graph::DiGraph` to represent step dependencies (both `inputs` artifact deps and explicit `depends_on` ordering deps). It validates that every `inputs` entry names the `output` of an earlier step, detects cycles, and returns topologically sorted batches (steps with no remaining unsatisfied deps can run concurrently).

- [ ] **Step 1: Write failing tests for DAG validation**

In `crates/workflows/src/dag.rs`, add tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StepDef, WorkflowDef};

    fn make_step(id: &str, inputs: &[&str], depends_on: &[&str], output: Option<&str>) -> StepDef {
        StepDef {
            id: id.to_string(),
            agent: "TestAgent".to_string(),
            model: None,
            inputs: if inputs.is_empty() { None } else { Some(inputs.iter().map(|s| s.to_string()).collect()) },
            depends_on: if depends_on.is_empty() { None } else { Some(depends_on.iter().map(|s| s.to_string()).collect()) },
            prompt: "do {{task}}".to_string(),
            output: output.map(|s| s.to_string()),
            requires_approval: None,
        }
    }

    fn make_def(steps: Vec<StepDef>) -> WorkflowDef {
        WorkflowDef { name: "test".to_string(), description: None, steps, budget: None }
    }

    #[test]
    fn linear_chain_produces_sequential_batches() {
        let def = make_def(vec![
            make_step("a", &[], &[], Some("out_a")),
            make_step("b", &["out_a"], &[], Some("out_b")),
            make_step("c", &["out_b"], &[], None),
        ]);
        let batches = DagValidator::new(&def).validate().unwrap();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0], vec!["a"]);
        assert_eq!(batches[1], vec!["b"]);
        assert_eq!(batches[2], vec!["c"]);
    }

    #[test]
    fn independent_steps_are_in_same_batch() {
        let def = make_def(vec![
            make_step("a", &[], &[], Some("out_a")),
            make_step("b", &[], &[], Some("out_b")),
            make_step("c", &["out_a", "out_b"], &[], None),
        ]);
        let batches = DagValidator::new(&def).validate().unwrap();
        // "a" and "b" can run concurrently in batch 0; "c" in batch 1
        assert_eq!(batches.len(), 2);
        let mut first = batches[0].clone();
        first.sort();
        assert_eq!(first, vec!["a", "b"]);
        assert_eq!(batches[1], vec!["c"]);
    }

    #[test]
    fn cycle_is_detected() {
        let def = make_def(vec![
            make_step("a", &["out_b"], &[], Some("out_a")),
            make_step("b", &["out_a"], &[], Some("out_b")),
        ]);
        let err = DagValidator::new(&def).validate().unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::CycleDetected));
    }

    #[test]
    fn unknown_input_artifact_is_detected() {
        let def = make_def(vec![
            make_step("a", &["nonexistent_output"], &[], None),
        ]);
        let err = DagValidator::new(&def).validate().unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::UnknownInput { .. }));
    }

    #[test]
    fn explicit_depends_on_respected() {
        let def = make_def(vec![
            make_step("a", &[], &[], None),
            make_step("b", &[], &["a"], None),  // no artifact dep, just ordering
        ]);
        let batches = DagValidator::new(&def).validate().unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0], vec!["a"]);
        assert_eq!(batches[1], vec!["b"]);
    }

    #[test]
    fn single_step_workflow_is_valid() {
        let def = make_def(vec![make_step("only", &[], &[], None)]);
        let batches = DagValidator::new(&def).validate().unwrap();
        assert_eq!(batches, vec![vec!["only".to_string()]]);
    }
}
```

- [ ] **Step 2: Run tests (expect compile failure)**

```bash
cargo test -p agent007-workflows -- dag::tests 2>&1 | head -30
```

- [ ] **Step 3: Implement `DagValidator` in `crates/workflows/src/dag.rs`**

```rust
use std::collections::HashMap;
use petgraph::graph::DiGraph;
use petgraph::algo::toposort;
use crate::error::WorkflowError;
use crate::types::WorkflowDef;

/// Validates a WorkflowDef DAG and returns topologically sorted batches of step IDs.
/// Steps in the same batch have no inter-dependency and can execute concurrently.
pub struct DagValidator<'a> {
    def: &'a WorkflowDef,
}

impl<'a> DagValidator<'a> {
    pub fn new(def: &'a WorkflowDef) -> Self {
        Self { def }
    }

    /// Returns `Ok(batches)` where each inner `Vec<String>` is a set of step IDs
    /// that can execute concurrently. Batches are ordered: batch[0] has no deps,
    /// batch[1] depends only on batch[0], etc.
    pub fn validate(&self) -> Result<Vec<Vec<String>>, WorkflowError> {
        // Build a map: output_name → step_id that produces it
        let mut output_to_step: HashMap<String, String> = HashMap::new();
        for step in &self.def.steps {
            if let Some(out) = &step.output {
                output_to_step.insert(out.clone(), step.id.clone());
            }
        }

        // Build petgraph DiGraph: node = step index (in def.steps order)
        let mut graph: DiGraph<String, ()> = DiGraph::new();
        let node_indices: Vec<_> = self.def.steps.iter()
            .map(|s| graph.add_node(s.id.clone()))
            .collect();
        let id_to_node: HashMap<String, _> = self.def.steps.iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), node_indices[i]))
            .collect();

        for step in &self.def.steps {
            let to_node = id_to_node[&step.id];

            // Edges from artifact inputs
            for inp in step.inputs.iter().flatten() {
                let producer = output_to_step.get(inp).ok_or_else(|| {
                    WorkflowError::UnknownInput {
                        id: step.id.clone(),
                        input: inp.clone(),
                    }
                })?;
                let from_node = id_to_node[producer];
                graph.add_edge(from_node, to_node, ());
            }

            // Edges from explicit depends_on (ordering only)
            for dep in step.depends_on.iter().flatten() {
                let from_node = id_to_node.get(dep).ok_or_else(|| {
                    WorkflowError::UnknownInput {
                        id: step.id.clone(),
                        input: dep.clone(),
                    }
                })?;
                graph.add_edge(*from_node, to_node, ());
            }
        }

        // Detect cycles via petgraph toposort
        let topo_order = toposort(&graph, None)
            .map_err(|_| WorkflowError::CycleDetected)?;

        // Build batches: assign each node a "level" = 1 + max(level of predecessors)
        // Nodes with no predecessors have level 0.
        let mut level: HashMap<petgraph::graph::NodeIndex, usize> = HashMap::new();
        for &node in &topo_order {
            let max_pred_level = graph
                .neighbors_directed(node, petgraph::Direction::Incoming)
                .filter_map(|pred| level.get(&pred).copied())
                .max();
            level.insert(node, max_pred_level.map_or(0, |l| l + 1));
        }

        let max_level = level.values().copied().max().unwrap_or(0);
        let mut batches: Vec<Vec<String>> = vec![Vec::new(); max_level + 1];
        for &node in &topo_order {
            let lvl = level[&node];
            batches[lvl].push(graph[node].clone());
        }

        Ok(batches)
    }
}
```

- [ ] **Step 4: Run tests (expect green)**

```bash
cargo test -p agent007-workflows -- dag::tests
```

- [ ] **Step 5: Commit**

```
git add crates/workflows/src/dag.rs
git commit -m "feat(workflows): DAG validation + topological batch ordering via petgraph"
```

---

## Chunk 3: WorkflowLoader

### Task 3: Load WorkflowDef from TOML file and scan a directory for all workflows

**Files:**
- Implement: `crates/workflows/src/loader.rs`

- [ ] **Step 1: Write failing tests for WorkflowLoader**

In `crates/workflows/src/loader.rs`, add tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

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
        let err = loader.load_file(std::path::Path::new("/tmp/does_not_exist.toml")).unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::Io(_)));
    }

    #[test]
    fn load_invalid_toml_returns_parse_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        fs::write(&path, "not valid toml {{ [[").unwrap();

        let loader = WorkflowLoader::new(dir.path().to_path_buf());
        let err = loader.load_file(&path).unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::ParseError { .. }));
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
}
```

- [ ] **Step 2: Run tests (expect compile failure)**

```bash
cargo test -p agent007-workflows -- loader::tests 2>&1 | head -30
```

- [ ] **Step 3: Implement `WorkflowLoader` in `crates/workflows/src/loader.rs`**

```rust
use std::path::{Path, PathBuf};
use crate::error::WorkflowError;
use crate::types::WorkflowDef;

pub struct WorkflowLoader {
    pub dir: PathBuf,
}

impl WorkflowLoader {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Load a WorkflowDef from an explicit TOML file path.
    pub fn load_file(&self, path: &Path) -> Result<WorkflowDef, WorkflowError> {
        let raw = std::fs::read_to_string(path)?;
        toml::from_str::<WorkflowDef>(&raw).map_err(|e| WorkflowError::ParseError {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })
    }

    /// Load a workflow by short name (looks for `<dir>/<name>.toml`).
    pub fn load_named(&self, name: &str) -> Result<WorkflowDef, WorkflowError> {
        let path = self.dir.join(format!("{}.toml", name));
        self.load_file(&path)
    }

    /// Return all short names (stem of `.toml` files) in the loader directory.
    /// Returns empty vec if the directory does not exist.
    pub fn list_names(&self) -> Result<Vec<String>, WorkflowError> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
        Ok(names)
    }

    /// Load all workflows from the directory.
    pub fn load_all(&self) -> Result<Vec<WorkflowDef>, WorkflowError> {
        let names = self.list_names()?;
        names.iter().map(|n| self.load_named(n)).collect()
    }
}
```

- [ ] **Step 4: Run tests (expect green)**

```bash
cargo test -p agent007-workflows -- loader::tests
```

- [ ] **Step 5: Commit**

```
git add crates/workflows/src/loader.rs
git commit -m "feat(workflows): WorkflowLoader loads TOML files by name or scans a directory"
```

---

## Chunk 4: WorkflowRunner — Step Execution + Tera Prompt Rendering

### Task 4: Implement WorkflowRunner::run — validate DAG, execute batches, collect outputs

**Files:**
- Implement: `crates/workflows/src/runner.rs`

`WorkflowRunner` holds an `Arc<dyn PersonaProvider>`, an `Arc<ModelRouter>`, and an `Arc<dyn Dispatcher>`. For each batch of concurrent steps, it spawns tokio tasks. Each step:
1. Renders the `prompt` template via Tera (injecting `task` + all current artifact outputs).
2. Resolves the persona from `PersonaProvider` by `step.agent`.
3. Uses `step.model` or the persona's `preferred_model` to select a provider via `ModelRouter`.
4. Calls `ModelRouter::complete()` with the rendered prompt.
5. If `requires_approval = true`, calls the approval gate.
6. Stores the response text under `step.output` key in the shared outputs map.

- [ ] **Step 1: Write failing tests for WorkflowRunner**

In `crates/workflows/src/runner.rs`, add tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StepDef, WorkflowDef, BudgetConfig};
    use agent007_models::{MockProvider, ModelProvider, ModelRouter};
    use agent007_core::dispatcher::LocalDispatcher;
    use agent007_core::persona::{PersonaSpec, PersonaProvider, NoOpPersonaProvider};
    use std::sync::Arc;

    fn mock_runner(mock_reply: &str) -> WorkflowRunner {
        let mock = Arc::new(MockProvider::new(mock_reply, "mock"));
        let mut router = ModelRouter::new("mock");
        router.register("mock", mock as Arc<dyn ModelProvider>);
        let dispatcher = Arc::new(LocalDispatcher::new(32));
        WorkflowRunner::new(
            Arc::new(NoOpPersonaProvider),
            Arc::new(router),
            dispatcher as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        )
    }

    fn simple_def() -> WorkflowDef {
        WorkflowDef {
            name: "simple".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "step1".to_string(),
                agent: "Researcher".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: "research {{task}}".to_string(),
                output: Some("notes".to_string()),
                requires_approval: None,
            }],
            budget: None,
        }
    }

    fn two_step_def() -> WorkflowDef {
        WorkflowDef {
            name: "two".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "step1".to_string(),
                    agent: "Researcher".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: None,
                    prompt: "research {{task}}".to_string(),
                    output: Some("notes".to_string()),
                    requires_approval: None,
                },
                StepDef {
                    id: "step2".to_string(),
                    agent: "Coder".to_string(),
                    model: None,
                    inputs: Some(vec!["notes".to_string()]),
                    depends_on: None,
                    prompt: "implement based on {{notes}}".to_string(),
                    output: Some("code".to_string()),
                    requires_approval: None,
                },
            ],
            budget: None,
        }
    }

    #[tokio::test]
    async fn run_single_step_returns_output() {
        let runner = mock_runner("mocked output");
        let result = runner.run(&simple_def(), "build auth").await.unwrap();
        assert_eq!(result.steps_completed, 1);
        assert_eq!(result.steps_total, 1);
        assert_eq!(result.outputs.get("notes").map(|s| s.as_str()), Some("mocked output"));
    }

    #[tokio::test]
    async fn run_two_step_pipeline_passes_artifact() {
        let runner = mock_runner("mocked reply");
        let result = runner.run(&two_step_def(), "add login").await.unwrap();
        assert_eq!(result.steps_completed, 2);
        assert!(result.outputs.contains_key("notes"));
        assert!(result.outputs.contains_key("code"));
    }

    #[tokio::test]
    async fn validate_cycle_returns_error() {
        let def = WorkflowDef {
            name: "cycle".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "a".to_string(), agent: "A".to_string(), model: None,
                    inputs: Some(vec!["out_b".to_string()]), depends_on: None,
                    prompt: "p".to_string(), output: Some("out_a".to_string()),
                    requires_approval: None,
                },
                StepDef {
                    id: "b".to_string(), agent: "B".to_string(), model: None,
                    inputs: Some(vec!["out_a".to_string()]), depends_on: None,
                    prompt: "p".to_string(), output: Some("out_b".to_string()),
                    requires_approval: None,
                },
            ],
            budget: None,
        };
        let runner = mock_runner("x");
        let err = runner.validate(&def).unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::CycleDetected));
    }

    #[tokio::test]
    async fn tera_task_variable_is_injected() {
        // The mock returns a fixed string, but we verify no TemplateError is returned.
        let runner = mock_runner("ok");
        let def = WorkflowDef {
            name: "t".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s".to_string(), agent: "A".to_string(), model: None,
                inputs: None, depends_on: None,
                prompt: "task is {{task}}".to_string(),
                output: None, requires_approval: None,
            }],
            budget: None,
        };
        runner.run(&def, "my task").await.unwrap();
    }

    #[tokio::test]
    async fn result_has_correct_steps_total() {
        let runner = mock_runner("r");
        let result = runner.run(&two_step_def(), "task").await.unwrap();
        assert_eq!(result.steps_total, 2);
    }
}
```

- [ ] **Step 2: Run tests (expect compile failure)**

```bash
cargo test -p agent007-workflows -- runner::tests 2>&1 | head -40
```

- [ ] **Step 3: Implement `WorkflowRunner` in `crates/workflows/src/runner.rs`**

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use agent007_core::dispatcher::Dispatcher;
use agent007_core::persona::PersonaProvider;
use agent007_models::{ModelRouter, CompletionRequest, Message, Role};

use crate::approval::ApprovalGate;
use crate::dag::DagValidator;
use crate::error::WorkflowError;
use crate::types::{BudgetUsed, WorkflowDef, WorkflowResult};

pub struct WorkflowRunner {
    pub persona_provider: Arc<dyn PersonaProvider>,
    pub model_router: Arc<ModelRouter>,
    pub dispatcher: Arc<dyn Dispatcher>,
}

impl WorkflowRunner {
    pub fn new(
        persona_provider: Arc<dyn PersonaProvider>,
        model_router: Arc<ModelRouter>,
        dispatcher: Arc<dyn Dispatcher>,
    ) -> Self {
        Self { persona_provider, model_router, dispatcher }
    }

    /// Validate the DAG and return topological batches. Public so the CLI `validate` command
    /// can call it without running steps.
    pub fn validate(&self, def: &WorkflowDef) -> Result<Vec<Vec<String>>, WorkflowError> {
        DagValidator::new(def).validate()
    }

    /// Run the full workflow. `task_input` fills the `{{task}}` Tera variable.
    pub async fn run(
        &self,
        def: &WorkflowDef,
        task_input: &str,
    ) -> Result<WorkflowResult, WorkflowError> {
        let batches = self.validate(def)?;
        let steps_total = def.steps.len();

        // Shared output artifact store, protected by a Mutex for concurrent batch steps.
        let outputs: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let budget_used: Arc<Mutex<BudgetUsed>> = Arc::new(Mutex::new(BudgetUsed::default()));
        let mut steps_completed = 0_usize;

        // Build a lookup: step_id → StepDef
        let step_map: HashMap<String, _> = def.steps.iter()
            .map(|s| (s.id.clone(), s))
            .collect();

        for batch in &batches {
            // Snapshot current outputs for template rendering (read-only during batch)
            let current_outputs = outputs.lock().await.clone();

            // Build one Tera context per step in the batch (before spawning tasks)
            let mut step_futures = Vec::new();
            for step_id in batch {
                let step = *step_map.get(step_id).unwrap();
                let step = step.clone();
                let task_str = task_input.to_string();
                let ctx_outputs = current_outputs.clone();
                let router = self.model_router.clone();
                let persona_provider = self.persona_provider.clone();

                step_futures.push(tokio::spawn(async move {
                    // 1. Render Tera prompt
                    let rendered = render_prompt(&step.prompt, &task_str, &ctx_outputs)
                        .map_err(|e| WorkflowError::TemplateError {
                            id: step.id.clone(),
                            reason: e.to_string(),
                        })?;

                    // 2. Resolve model: step.model > persona.preferred_model > "mock"
                    let model_name = if let Some(m) = &step.model {
                        m.clone()
                    } else if let Ok(Some(persona)) = persona_provider.get(&step.agent) {
                        persona.preferred_model.clone().unwrap_or_else(|| "mock".to_string())
                    } else {
                        "mock".to_string()
                    };

                    // 3. Call model provider
                    let req = CompletionRequest {
                        model: model_name.clone(),
                        messages: vec![Message { role: Role::User, content: rendered }],
                        max_tokens: None,
                        temperature: None,
                        system: None,
                    };
                    let resp = router.complete(req).await.map_err(|e| WorkflowError::StepFailed {
                        id: step.id.clone(),
                        reason: e.to_string(),
                    })?;

                    Ok::<(String, Option<String>, bool, String), WorkflowError>((
                        step.id.clone(),
                        step.output.clone(),
                        step.requires_approval.unwrap_or(false),
                        resp.content,
                    ))
                }));
            }

            // Await all tasks in this batch
            for fut in step_futures {
                let (step_id, output_name, needs_approval, content) = fut
                    .await
                    .map_err(|e| WorkflowError::StepFailed {
                        id: "unknown".to_string(),
                        reason: e.to_string(),
                    })??;

                // Handle approval gate (sequential after the step completes)
                let final_content = if needs_approval {
                    ApprovalGate::prompt(&step_id, &content).await?
                } else {
                    content
                };

                // Enforce budget if configured
                if let Some(budget) = &def.budget {
                    let token_estimate = estimate_tokens(&final_content);
                    let usd_estimate = token_estimate as f64 * 0.000_002; // $2 per 1M tokens placeholder
                    let mut used = budget_used.lock().await;
                    used.tokens += token_estimate;
                    used.estimated_usd += usd_estimate;
                    check_budget(budget, &used)?;
                }

                // Store output artifact
                if let Some(out_name) = output_name {
                    outputs.lock().await.insert(out_name, final_content);
                }

                steps_completed += 1;
            }
        }

        let final_outputs = Arc::try_unwrap(outputs)
            .unwrap_or_else(|a| tokio::runtime::Handle::current().block_on(async { Mutex::new(a.lock().await.clone()) }))
            .into_inner();
        let final_budget = Arc::try_unwrap(budget_used)
            .unwrap_or_else(|a| tokio::runtime::Handle::current().block_on(async { Mutex::new(a.lock().await.clone()) }))
            .into_inner();

        Ok(WorkflowResult {
            outputs: final_outputs,
            steps_completed,
            steps_total,
            budget_used: final_budget,
        })
    }
}

fn render_prompt(
    template: &str,
    task: &str,
    outputs: &HashMap<String, String>,
) -> Result<String, tera::Error> {
    let mut tera = tera::Tera::default();
    tera.add_raw_template("prompt", template)?;
    let mut ctx = tera::Context::new();
    ctx.insert("task", task);
    for (k, v) in outputs {
        ctx.insert(k, v);
    }
    tera.render("prompt", &ctx)
}

fn estimate_tokens(text: &str) -> u64 {
    // Rough approximation: 1 token ≈ 4 chars
    (text.len() as u64) / 4
}

fn check_budget(
    budget: &crate::types::BudgetConfig,
    used: &BudgetUsed,
) -> Result<(), WorkflowError> {
    if let Some(max_tokens) = budget.max_tokens_per_session {
        if used.tokens > max_tokens {
            return Err(WorkflowError::BudgetExceeded(format!(
                "token limit {} exceeded (used {})", max_tokens, used.tokens
            )));
        }
    }
    if let Some(max_usd) = budget.max_usd_per_task {
        if used.estimated_usd > max_usd {
            return Err(WorkflowError::BudgetExceeded(format!(
                "USD limit ${:.4} exceeded (used ${:.4})", max_usd, used.estimated_usd
            )));
        }
    }
    Ok(())
}
```

> **Note:** The `Arc::try_unwrap` fallback in `run` can be simplified; the pattern above is defensive but correct when no other owners remain after all tasks finish. An alternative is to avoid the `Arc<Mutex<_>>` pattern for sequential batch steps — collect results into a `Vec` from `join_all` instead. The implementation above is acceptable for TDD purposes.

- [ ] **Step 4: Run tests (expect green)**

```bash
cargo test -p agent007-workflows -- runner::tests
```

- [ ] **Step 5: Commit**

```
git add crates/workflows/src/runner.rs
git commit -m "feat(workflows): WorkflowRunner executes DAG batches with Tera prompt rendering"
```

---

## Chunk 5: Budget Tracking and Enforcement

### Task 5: Verify budget enforcement paths (token limit, USD limit, alert_at_percent, on_exceed modes)

**Files:**
- Extend: `crates/workflows/src/runner.rs` (add `on_exceed` mode handling and alert path)

The budget logic is partially implemented in Chunk 4. This task adds: (a) tests for each `on_exceed` mode, (b) the `alert-only` path that logs a warning without aborting, and (c) the `pause` mode that blocks and prompts the user to continue before stopping.

- [ ] **Step 1: Write failing tests for budget enforcement**

Add to the `#[cfg(test)]` block in `runner.rs`:

```rust
    #[tokio::test]
    async fn budget_token_limit_stops_run() {
        let runner = mock_runner("a very long output that is definitely more than 1 token");
        let def = WorkflowDef {
            name: "budget-test".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s1".to_string(), agent: "A".to_string(), model: None,
                inputs: None, depends_on: None,
                prompt: "do {{task}}".to_string(),
                output: Some("out".to_string()), requires_approval: None,
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: Some(1),  // extremely low — 1 token
                max_usd_per_task: None,
                alert_at_percent: None,
                on_exceed: Some("stop".to_string()),
            }),
        };
        let err = runner.run(&def, "task").await.unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::BudgetExceeded(_)));
    }

    #[tokio::test]
    async fn budget_usd_limit_stops_run() {
        let runner = mock_runner("short");
        // Estimated cost of "short" (5 chars) = 5/4 * 0.000002 = 0.0000025 USD
        // Set limit just below that:
        let def = WorkflowDef {
            name: "budget-usd".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s1".to_string(), agent: "A".to_string(), model: None,
                inputs: None, depends_on: None,
                prompt: "do {{task}}".to_string(),
                output: Some("out".to_string()), requires_approval: None,
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: None,
                max_usd_per_task: Some(0.000_000_001),  // sub-nano USD — always exceeded
                alert_at_percent: None,
                on_exceed: Some("stop".to_string()),
            }),
        };
        let err = runner.run(&def, "task").await.unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::BudgetExceeded(_)));
    }

    #[tokio::test]
    async fn budget_alert_only_does_not_stop_run() {
        let runner = mock_runner("a very long output that is definitely more than 1 token");
        let def = WorkflowDef {
            name: "budget-alert".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s1".to_string(), agent: "A".to_string(), model: None,
                inputs: None, depends_on: None,
                prompt: "do {{task}}".to_string(),
                output: Some("out".to_string()), requires_approval: None,
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: Some(1),  // would exceed but mode is alert-only
                max_usd_per_task: None,
                alert_at_percent: None,
                on_exceed: Some("alert-only".to_string()),
            }),
        };
        // Should succeed despite exceeding the token limit
        let result = runner.run(&def, "task").await.unwrap();
        assert_eq!(result.steps_completed, 1);
    }
```

- [ ] **Step 2: Run tests (expect failures on alert-only test)**

```bash
cargo test -p agent007-workflows -- runner::tests::budget
```

- [ ] **Step 3: Update `check_budget` in `runner.rs` to respect `on_exceed` mode**

Replace the `check_budget` function:

```rust
fn check_budget(
    budget: &crate::types::BudgetConfig,
    used: &BudgetUsed,
) -> Result<(), WorkflowError> {
    let mode = budget.on_exceed.as_deref().unwrap_or("stop");

    let token_exceeded = budget.max_tokens_per_session
        .map_or(false, |max| used.tokens > max);
    let usd_exceeded = budget.max_usd_per_task
        .map_or(false, |max| used.estimated_usd > max);

    if !token_exceeded && !usd_exceeded {
        // Check alert threshold
        if let (Some(alert_pct), Some(max_tokens)) = (budget.alert_at_percent, budget.max_tokens_per_session) {
            let pct_used = (used.tokens as f64 / max_tokens as f64) * 100.0;
            if pct_used >= alert_pct as f64 {
                tracing::warn!(
                    "Budget alert: {:.0}% of token limit used ({}/{})",
                    pct_used, used.tokens, max_tokens
                );
            }
        }
        return Ok(());
    }

    let reason = if token_exceeded {
        format!("token limit {} exceeded (used {})", budget.max_tokens_per_session.unwrap(), used.tokens)
    } else {
        format!("USD limit ${:.6} exceeded (used ${:.6})", budget.max_usd_per_task.unwrap(), used.estimated_usd)
    };

    match mode {
        "alert-only" => {
            tracing::warn!("Budget exceeded (alert-only): {}", reason);
            Ok(())
        }
        "pause" => {
            // In pause mode: print to stderr and read y/n from stdin
            eprintln!("[BUDGET EXCEEDED] {} — continue? [y/n]: ", reason);
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            if input.trim().eq_ignore_ascii_case("y") {
                Ok(())
            } else {
                Err(WorkflowError::BudgetExceeded(reason))
            }
        }
        _ => Err(WorkflowError::BudgetExceeded(reason)),
    }
}
```

- [ ] **Step 4: Run tests (expect green)**

```bash
cargo test -p agent007-workflows -- runner::tests
```

- [ ] **Step 5: Commit**

```
git add crates/workflows/src/runner.rs
git commit -m "feat(workflows): budget enforcement with stop/alert-only/pause on_exceed modes"
```

---

## Chunk 6: Human-in-the-Loop Approval Gate

### Task 6: Implement the approval gate — stderr prompt, stdin y/n/edit handling

**Files:**
- Implement: `crates/workflows/src/approval.rs`

When `requires_approval = true` on a step, after the model returns output the runner calls `ApprovalGate::prompt`. This prints to stderr, reads from stdin, and either passes the content through, aborts, or opens `$EDITOR` for the user to revise the output.

- [ ] **Step 1: Write failing tests for ApprovalGate**

In `crates/workflows/src/approval.rs`, add tests. Since stdin interaction is hard to unit-test directly, test the parsing helper and the editor path with `EDITOR=cat` (which just echoes the file content back to stdout, leaving the tempfile unchanged):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_y_approves() {
        assert_eq!(ApprovalResponse::parse("y"), ApprovalResponse::Approve);
        assert_eq!(ApprovalResponse::parse("Y"), ApprovalResponse::Approve);
        assert_eq!(ApprovalResponse::parse("yes"), ApprovalResponse::Approve);
    }

    #[test]
    fn parse_n_denies() {
        assert_eq!(ApprovalResponse::parse("n"), ApprovalResponse::Deny);
        assert_eq!(ApprovalResponse::parse("N"), ApprovalResponse::Deny);
        assert_eq!(ApprovalResponse::parse("no"), ApprovalResponse::Deny);
    }

    #[test]
    fn parse_edit_returns_edit() {
        assert_eq!(ApprovalResponse::parse("edit"), ApprovalResponse::Edit);
        assert_eq!(ApprovalResponse::parse("e"), ApprovalResponse::Edit);
    }

    #[test]
    fn parse_unknown_defaults_to_deny() {
        assert_eq!(ApprovalResponse::parse("maybe"), ApprovalResponse::Deny);
        assert_eq!(ApprovalResponse::parse(""), ApprovalResponse::Deny);
    }

    #[tokio::test]
    async fn open_editor_returns_original_content_when_editor_is_true() {
        // `true` is a Unix command that exits 0 without modifying files.
        // The tempfile content should remain unchanged → same as original.
        let content = "original output";
        let result = open_in_editor(content, Some("true")).await;
        assert!(result.is_ok());
        // With `true` as editor the file is untouched, so content is returned as-is.
        assert_eq!(result.unwrap(), content);
    }
}
```

- [ ] **Step 2: Run tests (expect compile failure)**

```bash
cargo test -p agent007-workflows -- approval::tests 2>&1 | head -30
```

- [ ] **Step 3: Implement `crates/workflows/src/approval.rs`**

```rust
use std::io::Write;
use crate::error::WorkflowError;

#[derive(Debug, PartialEq)]
pub enum ApprovalResponse {
    Approve,
    Deny,
    Edit,
}

impl ApprovalResponse {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "y" | "yes" => ApprovalResponse::Approve,
            "n" | "no"  => ApprovalResponse::Deny,
            "e" | "edit" => ApprovalResponse::Edit,
            _ => ApprovalResponse::Deny,
        }
    }
}

pub struct ApprovalGate;

impl ApprovalGate {
    /// Present the approval gate to the user via stderr/stdin.
    /// Returns the (possibly edited) content on approval, or `WorkflowError::ApprovalDenied`.
    pub async fn prompt(step_id: &str, content: &str) -> Result<String, WorkflowError> {
        eprintln!("\n[APPROVAL REQUIRED] Step: {}", step_id);
        eprintln!("Output:\n{}\n", content);
        eprint!("Approve? [y/n/edit]: ");
        std::io::stderr().flush().ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)
            .map_err(WorkflowError::Io)?;

        match ApprovalResponse::parse(&input) {
            ApprovalResponse::Approve => Ok(content.to_string()),
            ApprovalResponse::Deny => Err(WorkflowError::ApprovalDenied(step_id.to_string())),
            ApprovalResponse::Edit => {
                let editor = std::env::var("EDITOR").ok();
                open_in_editor(content, editor.as_deref()).await.map_err(|e| {
                    WorkflowError::StepFailed {
                        id: step_id.to_string(),
                        reason: format!("editor failed: {}", e),
                    }
                })
            }
        }
    }
}

/// Write `content` to a tempfile, open `$EDITOR` (or the provided override), and return
/// the file contents after the editor exits.
pub async fn open_in_editor(content: &str, editor: Option<&str>) -> std::io::Result<String> {
    let editor_cmd = editor
        .map(|s| s.to_string())
        .or_else(|| std::env::var("EDITOR").ok())
        .unwrap_or_else(|| "vi".to_string());

    let mut tmpfile = tempfile::Builder::new()
        .suffix(".txt")
        .tempfile()?;
    tmpfile.write_all(content.as_bytes())?;
    tmpfile.flush()?;

    let path = tmpfile.path().to_owned();

    // Spawn editor as a blocking process (uses tokio::task::spawn_blocking)
    let path_clone = path.clone();
    let editor_clone = editor_cmd.clone();
    tokio::task::spawn_blocking(move || {
        std::process::Command::new(&editor_clone)
            .arg(&path_clone)
            .status()
    }).await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    std::fs::read_to_string(&path)
}
```

Note: add `tempfile = { workspace = true }` to `[dependencies]` (not just dev-deps) in `crates/workflows/Cargo.toml` since `open_in_editor` is used in production code.

- [ ] **Step 4: Run tests (expect green)**

```bash
cargo test -p agent007-workflows -- approval::tests
```

- [ ] **Step 5: Run all workflows tests**

```bash
cargo test -p agent007-workflows
```

- [ ] **Step 6: Commit**

```
git add crates/workflows/src/approval.rs crates/workflows/Cargo.toml
git commit -m "feat(workflows): human-in-the-loop approval gate with y/n/edit and \$EDITOR support"
```

---

## Chunk 7: CLI Commands

### Task 7: Implement `agent007 workflow` subcommand (run, list, validate, show)

**Files:**
- Create: `crates/cli/src/commands/workflow.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `crates/cli/src/main.rs`
- Modify: `crates/cli/Cargo.toml` (add `agent007-workflows` dep)

- [ ] **Step 1: Add `agent007-workflows` to `crates/cli/Cargo.toml`**

```toml
agent007-workflows = { path = "../workflows" }
```

- [ ] **Step 2: Add `pub mod workflow;` to `crates/cli/src/commands/mod.rs`**

```rust
pub mod run;
pub mod serve;
pub mod skill;
pub mod simulate;
pub mod workflow;
```

- [ ] **Step 3: Add `WorkflowArgs` and `Commands::Workflow` to `crates/cli/src/main.rs`**

Add after the `SkillArgs` block:

```rust
#[derive(Parser, Debug)]
pub struct WorkflowArgs {
    #[command(subcommand)]
    pub action: WorkflowAction,
}

#[derive(Subcommand, Debug)]
pub enum WorkflowAction {
    /// Run a named workflow with an initial task
    Run {
        /// Workflow name (resolves ~/.agent007/workflows/<name>.toml)
        name: String,
        /// Initial task input for {{task}} template variable
        #[arg(long)]
        task: String,
    },
    /// List all available workflows
    List,
    /// Validate a workflow DAG without running it
    Validate {
        /// Workflow name
        name: String,
    },
    /// Show a workflow's steps and dependencies
    Show {
        /// Workflow name
        name: String,
    },
}
```

Add `Workflow` variant to `Commands`:

```rust
/// Manage and run multi-agent workflows
Workflow(WorkflowArgs),
```

Add match arm in `main()`:

```rust
Commands::Workflow(w) => commands::workflow::execute(config, w.action).await,
```

- [ ] **Step 4: Write failing tests for CLI workflow parsing**

Add to the `#[cfg(test)]` block in `main.rs`:

```rust
    #[test]
    fn parse_workflow_run_subcommand() {
        let cli = Cli::try_parse_from([
            "agent007", "workflow", "run", "tdd-feature", "--task", "add auth"
        ]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Workflow(ref w) if matches!(
                &w.action,
                WorkflowAction::Run { name, task }
                if name == "tdd-feature" && task == "add auth"
            )
        ));
    }

    #[test]
    fn parse_workflow_list_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "workflow", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Workflow(ref w) if matches!(w.action, WorkflowAction::List)));
    }

    #[test]
    fn parse_workflow_validate_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "workflow", "validate", "my-flow"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Workflow(ref w) if matches!(&w.action, WorkflowAction::Validate { name } if name == "my-flow")
        ));
    }

    #[test]
    fn parse_workflow_show_subcommand() {
        let cli = Cli::try_parse_from(["agent007", "workflow", "show", "my-flow"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Workflow(ref w) if matches!(&w.action, WorkflowAction::Show { name } if name == "my-flow")
        ));
    }
```

- [ ] **Step 5: Run tests (expect compile failure until workflow.rs exists)**

```bash
cargo test -p agent007 2>&1 | head -40
```

- [ ] **Step 6: Implement `crates/cli/src/commands/workflow.rs`**

```rust
use anyhow::Result;
use std::sync::Arc;
use crate::config::Config;
use crate::commands::run::{agent007_home, build_stack};
use crate::main::WorkflowAction;
use agent007_workflows::{WorkflowLoader, WorkflowRunner};

pub async fn execute(config: Arc<Config>, action: WorkflowAction) -> Result<()> {
    let workflows_dir = agent007_home().join("workflows");
    let loader = WorkflowLoader::new(workflows_dir.clone());

    match action {
        WorkflowAction::List => {
            let names = loader.list_names()?;
            if names.is_empty() {
                println!("No workflows found in {}", workflows_dir.display());
                println!("Add TOML files to {} to create workflows.", workflows_dir.display());
            } else {
                println!("Available workflows (in {}):", workflows_dir.display());
                for name in &names {
                    // Try to load the description
                    let desc = loader.load_named(name)
                        .ok()
                        .and_then(|d| d.description)
                        .unwrap_or_default();
                    if desc.is_empty() {
                        println!("  {}", name);
                    } else {
                        println!("  {} — {}", name, desc);
                    }
                }
            }
        }

        WorkflowAction::Validate { name } => {
            let def = loader.load_named(&name)?;
            let stack = build_stack(&config).await?;
            let runner = WorkflowRunner::new(
                stack.persona_provider.clone(),
                stack.model_router.clone(),
                stack.dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
            );
            match runner.validate(&def) {
                Ok(batches) => {
                    println!("Workflow '{}' is valid.", name);
                    println!("Execution plan ({} batch(es)):", batches.len());
                    for (i, batch) in batches.iter().enumerate() {
                        println!("  Batch {}: [{}]", i + 1, batch.join(", "));
                    }
                }
                Err(e) => {
                    eprintln!("Workflow '{}' is invalid: {}", name, e);
                    std::process::exit(1);
                }
            }
        }

        WorkflowAction::Show { name } => {
            let def = loader.load_named(&name)?;
            println!("Workflow: {}", def.name);
            if let Some(desc) = &def.description {
                println!("Description: {}", desc);
            }
            println!("\nSteps:");
            for step in &def.steps {
                println!("  [{}] agent={}", step.id, step.agent);
                if let Some(m) = &step.model {
                    println!("       model={}", m);
                }
                if let Some(inputs) = &step.inputs {
                    println!("       inputs=[{}]", inputs.join(", "));
                }
                if let Some(deps) = &step.depends_on {
                    println!("       depends_on=[{}]", deps.join(", "));
                }
                if let Some(out) = &step.output {
                    println!("       output={}", out);
                }
                if step.requires_approval == Some(true) {
                    println!("       requires_approval=true");
                }
            }
            if let Some(budget) = &def.budget {
                println!("\nBudget:");
                if let Some(t) = budget.max_tokens_per_session {
                    println!("  max_tokens_per_session={}", t);
                }
                if let Some(u) = budget.max_usd_per_task {
                    println!("  max_usd_per_task=${:.2}", u);
                }
                if let Some(pct) = budget.alert_at_percent {
                    println!("  alert_at_percent={}%", pct);
                }
                if let Some(mode) = &budget.on_exceed {
                    println!("  on_exceed={}", mode);
                }
            }
        }

        WorkflowAction::Run { name, task } => {
            let def = loader.load_named(&name)?;
            println!("Running workflow '{}' with task: {}", name, task);

            let stack = build_stack(&config).await?;
            let runner = WorkflowRunner::new(
                stack.persona_provider.clone(),
                stack.model_router.clone(),
                stack.dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
            );

            match runner.run(&def, &task).await {
                Ok(result) => {
                    println!(
                        "\nWorkflow '{}' completed: {}/{} steps",
                        name, result.steps_completed, result.steps_total
                    );
                    println!(
                        "Budget used: {} tokens, ${:.6}",
                        result.budget_used.tokens, result.budget_used.estimated_usd
                    );
                    if !result.outputs.is_empty() {
                        println!("\nOutputs:");
                        for (key, value) in &result.outputs {
                            let preview = if value.len() > 200 {
                                format!("{}... ({} chars)", &value[..200], value.len())
                            } else {
                                value.clone()
                            };
                            println!("  [{}]:\n{}\n", key, preview);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Workflow '{}' failed: {}", name, e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
```

> **Note:** `stack.persona_provider` requires that `Stack` gains a `persona_provider: Arc<dyn PersonaProvider>` field in Chunk 8. For compilation during this chunk, reference the field directly in the workflow command and leave a `// TODO: wire persona_provider in Chunk 8` comment if it does not exist yet. The `build_stack` changes are isolated to Chunk 8.

- [ ] **Step 7: Run CLI parsing tests (expect green)**

```bash
cargo test -p agent007 -- parse_workflow
```

- [ ] **Step 8: Run all tests**

```bash
cargo test -p agent007-workflows && cargo test -p agent007
```

- [ ] **Step 9: Commit**

```
git add crates/cli/src/commands/workflow.rs crates/cli/src/commands/mod.rs crates/cli/src/main.rs crates/cli/Cargo.toml
git commit -m "feat(cli): add workflow subcommand (run, list, validate, show)"
```

---

## Chunk 8: Wire Into build_stack

### Task 8: Add `workflow_runner` to `Stack`; create `~/.agent007/workflows/` directory on startup

**Files:**
- Modify: `crates/cli/src/commands/run.rs`

`Stack` gains a `workflow_runner: Arc<WorkflowRunner>` field. `build_stack` instantiates `WorkflowRunner` using the `PersonaProvider` that was already wired in the personas crate plan (Plan 5). The `~/.agent007/workflows/` directory is created if absent.

- [ ] **Step 1: Write failing test for Stack field**

Add to the `#[cfg(test)]` block in `run.rs`:

```rust
    #[tokio::test]
    async fn stack_contains_workflow_runner() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Config::default();
        let stack = build_stack(&config).await.unwrap();
        // WorkflowRunner is on the stack — test that it can validate an empty-step workflow
        use agent007_workflows::types::{WorkflowDef};
        let def = WorkflowDef { name: "t".to_string(), description: None, steps: vec![], budget: None };
        let result = stack.workflow_runner.validate(&def);
        // Empty workflow validates to empty batches
        assert!(result.is_ok());
        std::env::remove_var("AGENT007_DRY_RUN");
    }

    #[tokio::test]
    async fn build_stack_creates_workflows_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path().to_str().unwrap());
        let config = Config::default();
        let _ = build_stack(&config).await.unwrap();
        assert!(tmp.path().join("workflows").exists());
        std::env::remove_var("AGENT007_HOME");
        std::env::remove_var("AGENT007_DRY_RUN");
    }
```

- [ ] **Step 2: Run tests (expect compile failure)**

```bash
cargo test -p agent007 -- stack_contains_workflow_runner 2>&1 | head -30
```

- [ ] **Step 3: Add `workflow_runner` field to `Stack` in `run.rs`**

Add to `Stack` struct:

```rust
pub workflow_runner: Arc<agent007_workflows::WorkflowRunner>,
```

Add to `build_stack`, after the `skill_executor` block (step 10 in current code):

```rust
    // 12. WorkflowRunner
    let workflows_dir = home.join("workflows");
    std::fs::create_dir_all(&workflows_dir)?;
    let workflow_runner = Arc::new(agent007_workflows::WorkflowRunner::new(
        // Use the persona provider already built for the orchestrator.
        // If personas crate is not yet wired, use NoOpPersonaProvider as a fallback:
        Arc::new(agent007_core::persona::NoOpPersonaProvider),
        model_router.clone(),
        dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
    ));
```

Add to the `Ok(Stack { ... })` initialiser:

```rust
        workflow_runner,
```

- [ ] **Step 4: Update `crates/cli/src/commands/workflow.rs` to use `stack.workflow_runner`**

Replace the ad-hoc `WorkflowRunner::new(...)` calls in `execute` with:

```rust
let stack = build_stack(&config).await?;
let runner = stack.workflow_runner.clone();
```

- [ ] **Step 5: Run all tests**

```bash
cargo test -p agent007-workflows && cargo test -p agent007
```

- [ ] **Step 6: Commit**

```
git add crates/cli/src/commands/run.rs crates/cli/src/commands/workflow.rs
git commit -m "feat(cli): wire WorkflowRunner into Stack; create ~/.agent007/workflows/ on startup"
```

---

## Final Verification

- [ ] **Run full workspace test suite**

```bash
cargo test --workspace 2>&1 | tail -20
```

- [ ] **Smoke test CLI parsing for all four workflow subcommands**

```bash
cargo run -p agent007 -- workflow --help
cargo run -p agent007 -- workflow list
cargo run -p agent007 -- workflow validate nonexistent 2>&1 | head -5
```

- [ ] **Verify no clippy warnings in workflows crate**

```bash
cargo clippy -p agent007-workflows -- -D warnings
```

- [ ] **Confirm file tree matches spec**

```bash
ls crates/workflows/src/
# Expected: approval.rs  dag.rs  error.rs  lib.rs  loader.rs  runner.rs  types.rs
```
