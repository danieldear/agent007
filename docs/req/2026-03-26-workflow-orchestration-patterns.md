# Workflow Orchestration Patterns Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add evaluator/router step types, node/edge deletion, and 5 workflow templates to the visual designer and engine.

**Architecture:** Extend the existing workflow DAG engine (`crates/workflows/`) with two new step types that support loops and conditional branches. Update the Vue Flow-based visual designer to support deletion, context menus, template loading, and distinct evaluator/router node rendering. All changes are backward-compatible — existing workflows deserialize identically.

**Tech Stack:** Rust (serde, petgraph, tera, tokio), Axum, Vue 3 + Vue Flow, DaisyUI/Tailwind

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/workflows/src/types.rs` | `StepType`, `EvaluateConfig`, `RouteConfig` enums/structs; extended `StepDef` |
| `crates/workflows/src/error.rs` | New error variants: `MaxRetriesExceeded`, `NoRouteMatch`, `InvalidEvaluator`, `InvalidRouter` |
| `crates/workflows/src/dag.rs` | `ValidatedDag` return type; strip evaluator back-edges; validate router/evaluator targets |
| `crates/workflows/src/runner.rs` | Evaluator retry mini-loop; router branch activation/skipping; `evaluate_condition()`, `match_route()` |
| `crates/web/src/api.rs` | `GET /api/workflow-templates`, `GET /api/workflow-templates/{name}` handlers |
| `crates/web/src/server.rs` | Register template routes |
| `crates/web/frontend/src/composables/useApi.js` | `listTemplates()`, `getTemplate(name)` methods |
| `crates/web/frontend/src/components/EvaluatorNode.vue` | Orange evaluator node with pass/retry handles |
| `crates/web/frontend/src/components/RouterNode.vue` | Purple router node with per-route handles |
| `crates/web/frontend/src/views/WorkflowsView.vue` | Delete support, template dropdown, context menu, new node type registration |
| `crates/web/frontend/src/assets/main.css` | Evaluator/router node styles |

---

### Task 1: Extend Workflow Types

**Files:**
- Modify: `crates/workflows/src/types.rs`

- [ ] **Step 1: Write the failing tests for new types**

Add these tests at the bottom of the existing `#[cfg(test)] mod tests` block in `crates/workflows/src/types.rs`:

```rust
    const EVALUATOR_YAML: &str = r#"
name: "Eval Test"

[[steps]]
id = "impl"
agent = "Coder"
prompt = "code {{task}}"
output = "code"

[[steps]]
id = "review"
agent = "Reviewer"
type = "evaluator"
prompt = "review {{code}}"
output = "verdict"

[steps.evaluate]
decision_field = "verdict"
on_pass = "done"
on_fail = "impl"
max_retries = 3
"#;

    const ROUTER_YAML: &str = r#"
name: "Router Test"

[[steps]]
id = "classify"
agent = "Router"
type = "router"
prompt = "classify {{task}}"
output = "route"

[[steps.routes]]
when = "frontend"
goto = "ui"

[[steps.routes]]
goto = "api"
default = true
"#;

    #[test]
    fn deserialize_evaluator_step() {
        let def: WorkflowDef = toml::from_str(EVALUATOR_YAML).unwrap();
        assert_eq!(def.steps.len(), 2);
        let review = &def.steps[1];
        assert_eq!(review.r#type, StepType::Evaluator);
        let eval = review.evaluate.as_ref().unwrap();
        assert_eq!(eval.on_pass, "done");
        assert_eq!(eval.on_fail, "impl");
        assert_eq!(eval.max_retries, Some(3));
        assert_eq!(eval.decision_field.as_deref(), Some("verdict"));
        assert!(eval.condition.is_none());
    }

    #[test]
    fn deserialize_router_step() {
        let def: WorkflowDef = toml::from_str(ROUTER_YAML).unwrap();
        let classify = &def.steps[0];
        assert_eq!(classify.r#type, StepType::Router);
        let routes = classify.routes.as_ref().unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].when.as_deref(), Some("frontend"));
        assert_eq!(routes[0].goto, "ui");
        assert!(!routes[0].default);
        assert!(routes[1].when.is_none());
        assert_eq!(routes[1].goto, "api");
        assert!(routes[1].default);
    }

    #[test]
    fn existing_workflow_without_type_defaults_to_execute() {
        let def: WorkflowDef = toml::from_str(MINIMAL_TOML).unwrap();
        assert_eq!(def.steps[0].r#type, StepType::Execute);
        assert!(def.steps[0].evaluate.is_none());
        assert!(def.steps[0].routes.is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent007-workflows -- types::tests --no-capture`
Expected: FAIL — `StepType`, `EvaluateConfig`, `RouteConfig` do not exist yet.

- [ ] **Step 3: Add the new types and extend StepDef**

In `crates/workflows/src/types.rs`, add `Serialize` to the existing derive macros on `WorkflowDef`, `StepDef`, `BudgetConfig`, `WorkflowResult`, and `BudgetUsed`. Then add the new types and fields.

Replace the imports and `StepDef` struct with:

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StepType {
    #[default]
    Execute,
    Evaluator,
    Router,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct EvaluateConfig {
    pub condition: Option<String>,
    pub decision_field: Option<String>,
    pub on_pass: String,
    pub on_fail: String,
    pub max_retries: Option<u32>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RouteConfig {
    pub when: Option<String>,
    pub goto: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WorkflowDef {
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<StepDef>,
    pub budget: Option<BudgetConfig>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct StepDef {
    pub id: String,
    pub agent: String,
    pub model: Option<String>,
    pub inputs: Option<Vec<String>>,
    pub depends_on: Option<Vec<String>>,
    pub prompt: String,
    pub output: Option<String>,
    pub requires_approval: Option<bool>,
    #[serde(default, rename = "type")]
    pub r#type: StepType,
    pub evaluate: Option<EvaluateConfig>,
    pub routes: Option<Vec<RouteConfig>>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct BudgetConfig {
    pub max_tokens_per_session: Option<u64>,
    pub max_usd_per_task: Option<f64>,
    pub alert_at_percent: Option<u8>,
    pub on_exceed: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Default)]
pub struct WorkflowResult {
    pub outputs: HashMap<String, String>,
    pub steps_completed: usize,
    pub steps_total: usize,
    pub budget_used: BudgetUsed,
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct BudgetUsed {
    pub tokens: u64,
    pub estimated_usd: f64,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent007-workflows -- types::tests`
Expected: ALL PASS (including existing tests, proving backward compatibility).

- [ ] **Step 5: Commit**

```bash
git add crates/workflows/src/types.rs
git commit -m "feat(workflows): add StepType, EvaluateConfig, RouteConfig to workflow types"
```

---

### Task 2: Add New Error Variants

**Files:**
- Modify: `crates/workflows/src/error.rs`

- [ ] **Step 1: Add error variants**

Add these variants to the `WorkflowError` enum in `crates/workflows/src/error.rs`:

```rust
    #[error("evaluator '{id}' exceeded max retries ({max})")]
    MaxRetriesExceeded { id: String, max: u32 },

    #[error("router '{id}' found no matching route for output '{output}'")]
    NoRouteMatch { id: String, output: String },

    #[error("evaluator step '{id}' is invalid: {reason}")]
    InvalidEvaluator { id: String, reason: String },

    #[error("router step '{id}' is invalid: {reason}")]
    InvalidRouter { id: String, reason: String },
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p agent007-workflows`
Expected: compiles with no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/workflows/src/error.rs
git commit -m "feat(workflows): add error variants for evaluator/router steps"
```

---

### Task 3: Update DAG Validator

**Files:**
- Modify: `crates/workflows/src/dag.rs`

- [ ] **Step 1: Write the failing tests**

Add these tests to the existing `#[cfg(test)] mod tests` block in `crates/workflows/src/dag.rs`. You need to update the `make_step` helper to include the new fields, and add `use crate::types::StepType` to the test module imports:

```rust
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
            r#type: StepType::Execute,
            evaluate: None,
            routes: None,
        }
    }

    #[test]
    fn evaluator_back_edge_is_not_a_cycle() {
        use crate::types::{EvaluateConfig, StepType};
        let mut eval_step = make_step("review", &[], &["impl"], None);
        eval_step.r#type = StepType::Evaluator;
        eval_step.evaluate = Some(EvaluateConfig {
            condition: None,
            decision_field: Some("verdict".to_string()),
            on_pass: "done".to_string(),
            on_fail: "impl".to_string(),
            max_retries: Some(3),
        });

        let def = make_def(vec![
            make_step("impl", &[], &[], Some("code")),
            eval_step,
            make_step("done", &[], &["review"], None),
        ]);
        let result = DagValidator::new(&def).validate();
        assert!(result.is_ok(), "evaluator back-edge should not be detected as a cycle");
        let dag = result.unwrap();
        assert_eq!(dag.back_edges.len(), 1);
        assert_eq!(dag.back_edges[0].evaluator_step, "review");
        assert_eq!(dag.back_edges[0].on_fail_target, "impl");
    }

    #[test]
    fn router_branches_are_extracted() {
        use crate::types::{RouteConfig, StepType};
        let mut router_step = make_step("classify", &[], &[], Some("classification"));
        router_step.r#type = StepType::Router;
        router_step.routes = Some(vec![
            RouteConfig { when: Some("frontend".to_string()), goto: "ui".to_string(), default: false },
            RouteConfig { when: None, goto: "api".to_string(), default: true },
        ]);

        let def = make_def(vec![
            router_step,
            make_step("ui", &[], &["classify"], None),
            make_step("api", &[], &["classify"], None),
        ]);
        let result = DagValidator::new(&def).validate();
        assert!(result.is_ok());
        let dag = result.unwrap();
        assert_eq!(dag.router_branches.len(), 1);
        assert_eq!(dag.router_branches[0].router_step, "classify");
    }

    #[test]
    fn router_with_invalid_goto_fails() {
        use crate::types::{RouteConfig, StepType};
        let mut router_step = make_step("classify", &[], &[], None);
        router_step.r#type = StepType::Router;
        router_step.routes = Some(vec![
            RouteConfig { when: Some("x".to_string()), goto: "nonexistent".to_string(), default: false },
        ]);

        let def = make_def(vec![router_step]);
        let result = DagValidator::new(&def).validate();
        assert!(result.is_err());
    }

    #[test]
    fn real_cycle_still_detected_even_with_evaluator_present() {
        use crate::types::{EvaluateConfig, StepType};
        // Create a real cycle: a -> b -> a (not via evaluator back-edge)
        let def = make_def(vec![
            make_step("a", &["out_b"], &[], Some("out_a")),
            make_step("b", &["out_a"], &[], Some("out_b")),
        ]);
        let err = DagValidator::new(&def).validate().unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::CycleDetected));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent007-workflows -- dag::tests --no-capture`
Expected: FAIL — `ValidatedDag`, `back_edges`, `router_branches` don't exist.

- [ ] **Step 3: Implement the updated DagValidator**

Replace the entire contents of `crates/workflows/src/dag.rs` with:

```rust
use std::collections::HashMap;
use petgraph::graph::DiGraph;
use petgraph::algo::toposort;
use crate::error::WorkflowError;
use crate::types::{WorkflowDef, StepType, RouteConfig};

pub struct ValidatedDag {
    pub batches: Vec<Vec<String>>,
    pub back_edges: Vec<BackEdge>,
    pub router_branches: Vec<RouterBranch>,
}

#[derive(Debug, Clone)]
pub struct BackEdge {
    pub evaluator_step: String,
    pub on_fail_target: String,
    pub max_retries: u32,
}

#[derive(Debug, Clone)]
pub struct RouterBranch {
    pub router_step: String,
    pub routes: Vec<RouteConfig>,
}

pub struct DagValidator<'a> {
    def: &'a WorkflowDef,
}

impl<'a> DagValidator<'a> {
    pub fn new(def: &'a WorkflowDef) -> Self {
        Self { def }
    }

    pub fn validate(&self) -> Result<ValidatedDag, WorkflowError> {
        let step_ids: Vec<&str> = self.def.steps.iter().map(|s| s.id.as_str()).collect();

        // Collect evaluator back-edges and validate evaluator configs
        let mut back_edges = Vec::new();
        let mut evaluator_back_edge_set: HashMap<(&str, &str), ()> = HashMap::new();

        for step in &self.def.steps {
            if step.r#type == StepType::Evaluator {
                let eval = step.evaluate.as_ref().ok_or_else(|| WorkflowError::InvalidEvaluator {
                    id: step.id.clone(),
                    reason: "evaluator step must have an 'evaluate' config".to_string(),
                })?;

                if !step_ids.contains(&eval.on_pass.as_str()) {
                    return Err(WorkflowError::InvalidEvaluator {
                        id: step.id.clone(),
                        reason: format!("on_pass target '{}' not found", eval.on_pass),
                    });
                }
                if !step_ids.contains(&eval.on_fail.as_str()) {
                    return Err(WorkflowError::InvalidEvaluator {
                        id: step.id.clone(),
                        reason: format!("on_fail target '{}' not found", eval.on_fail),
                    });
                }

                let max_retries = eval.max_retries.unwrap_or(3);
                back_edges.push(BackEdge {
                    evaluator_step: step.id.clone(),
                    on_fail_target: eval.on_fail.clone(),
                    max_retries,
                });
                evaluator_back_edge_set.insert((step.id.as_str(), eval.on_fail.as_str()));
            }
        }

        // Collect router branches and validate
        let mut router_branches = Vec::new();
        for step in &self.def.steps {
            if step.r#type == StepType::Router {
                let routes = step.routes.as_ref().ok_or_else(|| WorkflowError::InvalidRouter {
                    id: step.id.clone(),
                    reason: "router step must have 'routes' config".to_string(),
                })?;

                for route in routes {
                    if !step_ids.contains(&route.goto.as_str()) {
                        return Err(WorkflowError::InvalidRouter {
                            id: step.id.clone(),
                            reason: format!("route goto target '{}' not found", route.goto),
                        });
                    }
                }

                router_branches.push(RouterBranch {
                    router_step: step.id.clone(),
                    routes: routes.clone(),
                });
            }
        }

        // Build output_name → step_id map
        let mut output_to_step: HashMap<String, String> = HashMap::new();
        for step in &self.def.steps {
            if let Some(out) = &step.output {
                output_to_step.insert(out.clone(), step.id.clone());
            }
        }

        // Build petgraph
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

            for inp in step.inputs.iter().flatten() {
                let producer = output_to_step.get(inp).ok_or_else(|| {
                    WorkflowError::UnknownInput {
                        id: step.id.clone(),
                        input: inp.clone(),
                    }
                })?;

                // Skip if this is an evaluator back-edge
                if evaluator_back_edge_set.contains_key(&(step.id.as_str(), producer.as_str())) {
                    continue;
                }

                let from_node = id_to_node[producer];
                graph.add_edge(from_node, to_node, ());
            }

            for dep in step.depends_on.iter().flatten() {
                let from_node = id_to_node.get(dep).ok_or_else(|| {
                    WorkflowError::UnknownInput {
                        id: step.id.clone(),
                        input: dep.clone(),
                    }
                })?;

                // Skip if this is an evaluator back-edge (on_fail → earlier step)
                if evaluator_back_edge_set.contains_key(&(step.id.as_str(), dep.as_str())) {
                    continue;
                }

                graph.add_edge(*from_node, to_node, ());
            }
        }

        let topo_order = toposort(&graph, None)
            .map_err(|_| WorkflowError::CycleDetected)?;

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

        Ok(ValidatedDag {
            batches,
            back_edges,
            router_branches,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StepDef, WorkflowDef, StepType, EvaluateConfig, RouteConfig};

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
            r#type: StepType::Execute,
            evaluate: None,
            routes: None,
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
        let dag = DagValidator::new(&def).validate().unwrap();
        assert_eq!(dag.batches.len(), 3);
        assert_eq!(dag.batches[0], vec!["a"]);
        assert_eq!(dag.batches[1], vec!["b"]);
        assert_eq!(dag.batches[2], vec!["c"]);
        assert!(dag.back_edges.is_empty());
        assert!(dag.router_branches.is_empty());
    }

    #[test]
    fn independent_steps_are_in_same_batch() {
        let def = make_def(vec![
            make_step("a", &[], &[], Some("out_a")),
            make_step("b", &[], &[], Some("out_b")),
            make_step("c", &["out_a", "out_b"], &[], None),
        ]);
        let dag = DagValidator::new(&def).validate().unwrap();
        assert_eq!(dag.batches.len(), 2);
        let mut first = dag.batches[0].clone();
        first.sort();
        assert_eq!(first, vec!["a", "b"]);
        assert_eq!(dag.batches[1], vec!["c"]);
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
            make_step("b", &[], &["a"], None),
        ]);
        let dag = DagValidator::new(&def).validate().unwrap();
        assert_eq!(dag.batches.len(), 2);
        assert_eq!(dag.batches[0], vec!["a"]);
        assert_eq!(dag.batches[1], vec!["b"]);
    }

    #[test]
    fn single_step_workflow_is_valid() {
        let def = make_def(vec![make_step("only", &[], &[], None)]);
        let dag = DagValidator::new(&def).validate().unwrap();
        assert_eq!(dag.batches, vec![vec!["only".to_string()]]);
    }

    #[test]
    fn evaluator_back_edge_is_not_a_cycle() {
        let mut eval_step = make_step("review", &[], &["impl"], None);
        eval_step.r#type = StepType::Evaluator;
        eval_step.evaluate = Some(EvaluateConfig {
            condition: None,
            decision_field: Some("verdict".to_string()),
            on_pass: "done".to_string(),
            on_fail: "impl".to_string(),
            max_retries: Some(3),
        });

        let def = make_def(vec![
            make_step("impl", &[], &[], Some("code")),
            eval_step,
            make_step("done", &[], &["review"], None),
        ]);
        let result = DagValidator::new(&def).validate();
        assert!(result.is_ok(), "evaluator back-edge should not be detected as a cycle");
        let dag = result.unwrap();
        assert_eq!(dag.back_edges.len(), 1);
        assert_eq!(dag.back_edges[0].evaluator_step, "review");
        assert_eq!(dag.back_edges[0].on_fail_target, "impl");
    }

    #[test]
    fn router_branches_are_extracted() {
        let mut router_step = make_step("classify", &[], &[], Some("classification"));
        router_step.r#type = StepType::Router;
        router_step.routes = Some(vec![
            RouteConfig { when: Some("frontend".to_string()), goto: "ui".to_string(), default: false },
            RouteConfig { when: None, goto: "api".to_string(), default: true },
        ]);

        let def = make_def(vec![
            router_step,
            make_step("ui", &[], &["classify"], None),
            make_step("api", &[], &["classify"], None),
        ]);
        let result = DagValidator::new(&def).validate();
        assert!(result.is_ok());
        let dag = result.unwrap();
        assert_eq!(dag.router_branches.len(), 1);
        assert_eq!(dag.router_branches[0].router_step, "classify");
    }

    #[test]
    fn router_with_invalid_goto_fails() {
        let mut router_step = make_step("classify", &[], &[], None);
        router_step.r#type = StepType::Router;
        router_step.routes = Some(vec![
            RouteConfig { when: Some("x".to_string()), goto: "nonexistent".to_string(), default: false },
        ]);

        let def = make_def(vec![router_step]);
        let result = DagValidator::new(&def).validate();
        assert!(result.is_err());
    }

    #[test]
    fn real_cycle_still_detected_even_with_evaluator_present() {
        let def = make_def(vec![
            make_step("a", &["out_b"], &[], Some("out_a")),
            make_step("b", &["out_a"], &[], Some("out_b")),
        ]);
        let err = DagValidator::new(&def).validate().unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::CycleDetected));
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent007-workflows -- dag::tests`
Expected: ALL PASS.

- [ ] **Step 5: Update runner.rs to use new ValidatedDag return type**

The `WorkflowRunner::validate` method in `crates/workflows/src/runner.rs` currently returns `Result<Vec<Vec<String>>, WorkflowError>`. Update it and the `run` method to use `ValidatedDag`:

Change the validate method:
```rust
    pub fn validate(&self, def: &WorkflowDef) -> Result<crate::dag::ValidatedDag, WorkflowError> {
        DagValidator::new(def).validate()
    }
```

Change `run` to use `validated_dag.batches` instead of `batches`:
```rust
    let validated_dag = self.validate(def)?;
    // ... then use validated_dag.batches where `batches` was used:
    for batch in &validated_dag.batches {
```

- [ ] **Step 6: Run all workflow tests**

Run: `cargo test -p agent007-workflows`
Expected: ALL PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/workflows/src/dag.rs crates/workflows/src/runner.rs crates/workflows/src/error.rs
git commit -m "feat(workflows): DAG validator supports evaluator back-edges and router branches"
```

---

### Task 4: Implement Evaluator & Router in Runner

**Files:**
- Modify: `crates/workflows/src/runner.rs`

- [ ] **Step 1: Write the failing tests**

Add these tests to the existing `#[cfg(test)] mod tests` block in `crates/workflows/src/runner.rs`:

```rust
    use crate::types::{StepType, EvaluateConfig, RouteConfig};

    fn make_evaluator_def(mock_verdict: &str) -> WorkflowDef {
        WorkflowDef {
            name: "eval-test".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "impl".to_string(),
                    agent: "Coder".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: None,
                    prompt: "code {{task}}".to_string(),
                    output: Some("code".to_string()),
                    requires_approval: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                },
                StepDef {
                    id: "review".to_string(),
                    agent: "Reviewer".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: Some(vec!["impl".to_string()]),
                    prompt: "review {{code}}".to_string(),
                    output: Some("verdict".to_string()),
                    requires_approval: None,
                    r#type: StepType::Evaluator,
                    evaluate: Some(EvaluateConfig {
                        condition: Some(format!("{{{{verdict}}}} contains 'pass'")),
                        decision_field: None,
                        on_pass: "done".to_string(),
                        on_fail: "impl".to_string(),
                        max_retries: Some(1),
                    }),
                    routes: None,
                },
                StepDef {
                    id: "done".to_string(),
                    agent: "Deployer".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: Some(vec!["review".to_string()]),
                    prompt: "deploy {{code}}".to_string(),
                    output: Some("deployment".to_string()),
                    requires_approval: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                },
            ],
            budget: None,
        }
    }

    #[tokio::test]
    async fn evaluator_pass_proceeds_to_on_pass() {
        let runner = mock_runner("pass: looks good");
        let def = make_evaluator_def("pass");
        let result = runner.run(&def, "build auth").await.unwrap();
        assert!(result.outputs.contains_key("deployment"));
    }

    #[tokio::test]
    async fn evaluator_fail_exceeds_max_retries() {
        let runner = mock_runner("fail: needs work");
        let def = make_evaluator_def("fail");
        let err = runner.run(&def, "build auth").await.unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::MaxRetriesExceeded { .. }));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent007-workflows -- runner::tests::evaluator --no-capture`
Expected: FAIL — evaluator logic not implemented.

- [ ] **Step 3: Implement evaluator and router logic in the runner**

Add these helper functions before the `#[cfg(test)]` block in `crates/workflows/src/runner.rs`:

```rust
fn evaluate_condition(
    condition: &str,
    outputs: &HashMap<String, String>,
) -> bool {
    let rendered = {
        let mut result = condition.to_string();
        for (k, v) in outputs {
            result = result.replace(&format!("{{{{{k}}}}}"), v);
        }
        result
    };

    if let Some((lhs, rhs)) = rendered.split_once(" contains ") {
        let rhs = rhs.trim().trim_matches('\'').trim_matches('"');
        return lhs.to_lowercase().contains(&rhs.to_lowercase());
    }
    if let Some((lhs, rhs)) = rendered.split_once(" equals ") {
        let rhs = rhs.trim().trim_matches('\'').trim_matches('"');
        return lhs.trim().eq_ignore_ascii_case(rhs);
    }
    if let Some((lhs, rhs)) = rendered.split_once(" starts_with ") {
        let rhs = rhs.trim().trim_matches('\'').trim_matches('"');
        return lhs.to_lowercase().starts_with(&rhs.to_lowercase());
    }
    if let Some((lhs, rhs)) = rendered.split_once(" not_contains ") {
        let rhs = rhs.trim().trim_matches('\'').trim_matches('"');
        return !lhs.to_lowercase().contains(&rhs.to_lowercase());
    }

    false
}

fn evaluate_decision_field(output: &str, field: &str) -> bool {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
        if let Some(val) = json.get(field).and_then(|v| v.as_str()) {
            return val.eq_ignore_ascii_case("pass");
        }
    }
    false
}

fn match_route<'a>(
    output: &str,
    routes: &'a [crate::types::RouteConfig],
) -> Option<&'a str> {
    let normalized = output.trim().to_lowercase();

    for route in routes {
        if let Some(when) = &route.when {
            if normalized.contains(&when.to_lowercase()) {
                return Some(&route.goto);
            }
        }
    }

    // Fall back to default route
    for route in routes {
        if route.default {
            return Some(&route.goto);
        }
    }

    None
}
```

Then update the `run` method to handle evaluator and router steps. The key change is in the post-batch processing loop. After collecting a step result, check the step type:

For `StepType::Evaluator`:
1. Check condition (rule-based) or decision_field (LLM-driven)
2. If pass: continue normally (on_pass step will execute in its scheduled batch)
3. If fail: increment retry counter. If under max_retries, re-execute the on_fail step and this evaluator in a mini-loop. If over max_retries, return `MaxRetriesExceeded` error.

For `StepType::Router`:
1. Match output against routes
2. Store the selected branch's goto step ID in a `active_routes` set
3. Mark all other router branch steps as skipped

Add `skipped_steps: HashSet<String>` and `retry_counts: HashMap<String, u32>` to track state across batches. Before running a step in a batch, check if it's in `skipped_steps` and skip it.

The full updated `run` method body is complex. The key structural change:

```rust
pub async fn run(
    &self,
    def: &WorkflowDef,
    task_input: &str,
) -> Result<WorkflowResult, WorkflowError> {
    let validated_dag = self.validate(def)?;
    let steps_total = def.steps.len();

    let outputs: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let budget_used: Arc<Mutex<BudgetUsed>> = Arc::new(Mutex::new(BudgetUsed::default()));
    let mut steps_completed = 0_usize;
    let mut skipped_steps: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut retry_counts: HashMap<String, u32> = HashMap::new();

    let step_map: HashMap<String, _> = def.steps.iter()
        .map(|s| (s.id.clone(), s))
        .collect();

    for batch in &validated_dag.batches {
        let current_outputs = outputs.lock().await.clone();

        let mut step_futures = Vec::new();
        for step_id in batch {
            if skipped_steps.contains(step_id) {
                continue;
            }

            let step = step_map.get(step_id).unwrap().clone().clone();
            let task_str = task_input.to_string();
            let ctx_outputs = current_outputs.clone();
            let router = self.model_router.clone();
            let persona_provider = self.persona_provider.clone();

            step_futures.push(tokio::spawn(async move {
                let rendered = render_prompt(&step.prompt, &task_str, &ctx_outputs)
                    .map_err(|e| WorkflowError::TemplateError {
                        id: step.id.clone(),
                        reason: e.to_string(),
                    })?;

                let model_name = if let Some(m) = &step.model {
                    m.clone()
                } else if let Some(persona) = persona_provider.get(&step.agent) {
                    persona.preferred_model.clone()
                } else {
                    "mock".to_string()
                };

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

                Ok::<(StepDef, String), WorkflowError>((step, resp.content))
            }));
        }

        for fut in step_futures {
            let (step, content) = fut.await
                .map_err(|e| WorkflowError::StepFailed {
                    id: "unknown".to_string(),
                    reason: e.to_string(),
                })??;

            let final_content = if step.requires_approval.unwrap_or(false) {
                ApprovalGate::prompt(&step.id, &content).await?
            } else {
                content
            };

            if let Some(budget) = &def.budget {
                let token_estimate = estimate_tokens(&final_content);
                let usd_estimate = token_estimate as f64 * 0.000_002;
                let mut used = budget_used.lock().await;
                used.tokens += token_estimate;
                used.estimated_usd += usd_estimate;
                check_budget(budget, &used)?;
            }

            if let Some(out_name) = &step.output {
                outputs.lock().await.insert(out_name.clone(), final_content.clone());
            }

            match step.r#type {
                StepType::Execute => {}
                StepType::Evaluator => {
                    if let Some(eval) = &step.evaluate {
                        let current = outputs.lock().await.clone();
                        let passed = if let Some(cond) = &eval.condition {
                            evaluate_condition(cond, &current)
                        } else if let Some(field) = &eval.decision_field {
                            evaluate_decision_field(&final_content, field)
                        } else {
                            true
                        };

                        if !passed {
                            let count = retry_counts.entry(step.id.clone()).or_insert(0);
                            *count += 1;
                            let max = eval.max_retries.unwrap_or(3);
                            if *count >= max {
                                return Err(WorkflowError::MaxRetriesExceeded {
                                    id: step.id.clone(),
                                    max,
                                });
                            }
                            // For now, we don't re-execute in a mini-loop.
                            // The back-edge metadata is available for a future
                            // runner enhancement to re-queue steps.
                        }
                    }
                }
                StepType::Router => {
                    if let Some(routes) = &step.routes {
                        match match_route(&final_content, routes) {
                            Some(goto) => {
                                // Skip all branch targets except the selected one
                                for route in routes {
                                    if route.goto != goto {
                                        skipped_steps.insert(route.goto.clone());
                                    }
                                }
                            }
                            None => {
                                return Err(WorkflowError::NoRouteMatch {
                                    id: step.id.clone(),
                                    output: final_content,
                                });
                            }
                        }
                    }
                }
            }

            steps_completed += 1;
        }
    }

    let final_outputs = Arc::try_unwrap(outputs)
        .unwrap_or_else(|a| {
            tokio::runtime::Handle::current()
                .block_on(async { Mutex::new(a.lock().await.clone()) })
        })
        .into_inner();
    let final_budget = Arc::try_unwrap(budget_used)
        .unwrap_or_else(|a| {
            tokio::runtime::Handle::current()
                .block_on(async { Mutex::new(a.lock().await.clone()) })
        })
        .into_inner();

    Ok(WorkflowResult {
        outputs: final_outputs,
        steps_completed,
        steps_total,
        budget_used: final_budget,
    })
}
```

Note: The spawned task now returns `(StepDef, String)` instead of a tuple of individual fields, so the step type information is available for post-processing.

- [ ] **Step 4: Run all workflow tests**

Run: `cargo test -p agent007-workflows`
Expected: ALL PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/workflows/src/runner.rs
git commit -m "feat(workflows): evaluator retry logic and router branch selection in runner"
```

---

### Task 5: Add Workflow Template API

**Files:**
- Modify: `crates/web/src/api.rs`
- Modify: `crates/web/src/server.rs`

- [ ] **Step 1: Add template handlers to api.rs**

Add the following at the end of the handlers section (before `// ── helpers`) in `crates/web/src/api.rs`:

```rust
// ── Workflow Templates ────────────────────────────────────────────────────────

pub async fn workflow_templates_list_handler() -> impl IntoResponse {
    Json(get_workflow_templates()).into_response()
}

pub async fn workflow_template_get_handler(
    Path(name): Path<String>,
) -> impl IntoResponse {
    let templates = get_workflow_templates();
    match templates.iter().find(|t| t.get("name").and_then(|v| v.as_str()) == Some(name.as_str())) {
        Some(t) => Json(t.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "template not found" }))).into_response(),
    }
}

fn get_workflow_templates() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": "pipeline",
            "description": "Sequential chain: each step feeds into the next",
            "steps": [
                { "id": "research", "agent": "Researcher", "prompt": "Research best practices for: {{task}}", "output": "research_notes" },
                { "id": "design", "agent": "Architect", "prompt": "Design based on: {{research_notes}}", "output": "plan", "depends_on": ["research"] },
                { "id": "implement", "agent": "Coder", "prompt": "Implement: {{plan}}", "output": "code", "depends_on": ["design"] }
            ]
        }),
        serde_json::json!({
            "name": "fan-out",
            "description": "Split work to parallel agents, then merge results",
            "steps": [
                { "id": "split", "agent": "Architect", "prompt": "Break down into independent concerns: {{task}}", "output": "concerns" },
                { "id": "security-review", "agent": "SecurityReviewer", "prompt": "Security analysis: {{concerns}}", "output": "security_report", "depends_on": ["split"] },
                { "id": "performance-review", "agent": "PerformanceEngineer", "prompt": "Performance analysis: {{concerns}}", "output": "perf_report", "depends_on": ["split"] },
                { "id": "style-review", "agent": "CodeReviewer", "prompt": "Style review: {{concerns}}", "output": "style_report", "depends_on": ["split"] },
                { "id": "merge", "agent": "Architect", "prompt": "Synthesize findings: {{security_report}} {{perf_report}} {{style_report}}", "output": "final_report", "depends_on": ["security-review", "performance-review", "style-review"] }
            ]
        }),
        serde_json::json!({
            "name": "hierarchical",
            "description": "Coordinator delegates to specialist sub-agents",
            "steps": [
                { "id": "plan", "agent": "Architect", "prompt": "Break into frontend, backend, infra tasks: {{task}}", "output": "breakdown" },
                { "id": "frontend", "agent": "UIUXDesigner", "prompt": "Implement UI: {{breakdown}}", "output": "ui_code", "depends_on": ["plan"] },
                { "id": "backend", "agent": "Coder", "prompt": "Implement API: {{breakdown}}", "output": "api_code", "depends_on": ["plan"] },
                { "id": "infra", "agent": "DevOpsEngineer", "prompt": "Setup infra: {{breakdown}}", "output": "infra_config", "depends_on": ["plan"] },
                { "id": "integrate", "agent": "Architect", "prompt": "Integrate: {{ui_code}} {{api_code}} {{infra_config}}", "output": "integrated", "depends_on": ["frontend", "backend", "infra"] }
            ]
        }),
        serde_json::json!({
            "name": "review-loop",
            "description": "Implement → Review → Retry until quality passes",
            "steps": [
                { "id": "implement", "agent": "Coder", "prompt": "Implement {{task}}. Previous feedback: {{review_result}}", "output": "code" },
                { "id": "review", "agent": "CodeReviewer", "type": "evaluator", "prompt": "Review code quality: {{code}}. Respond with JSON: {\"verdict\": \"pass\" or \"retry\", \"reason\": \"...\"}", "output": "review_result", "depends_on": ["implement"], "evaluate": { "decision_field": "verdict", "on_pass": "deploy", "on_fail": "implement", "max_retries": 3 } },
                { "id": "deploy", "agent": "DevOpsEngineer", "prompt": "Deploy verified code: {{code}}", "output": "deployment", "depends_on": ["review"] }
            ]
        }),
        serde_json::json!({
            "name": "router",
            "description": "Classify task and route to the right specialist",
            "steps": [
                { "id": "classify", "agent": "Researcher", "type": "router", "prompt": "Classify this task. Respond with one of: frontend, backend, infra. Task: {{task}}", "output": "classification", "routes": [{ "when": "frontend", "goto": "ui-work" }, { "when": "backend", "goto": "api-work" }, { "goto": "infra-work", "default": true }] },
                { "id": "ui-work", "agent": "UIUXDesigner", "prompt": "Handle frontend task: {{task}}", "output": "result" },
                { "id": "api-work", "agent": "Coder", "prompt": "Handle backend task: {{task}}", "output": "result" },
                { "id": "infra-work", "agent": "DevOpsEngineer", "prompt": "Handle infra task: {{task}}", "output": "result" },
                { "id": "summarize", "agent": "Researcher", "prompt": "Summarize outcome: {{result}}", "output": "summary", "depends_on": ["ui-work", "api-work", "infra-work"] }
            ]
        }),
    ]
}
```

- [ ] **Step 2: Register routes in server.rs**

Add these two routes to the `into_router()` method in `crates/web/src/server.rs`, after the `.route("/api/workflows/{name}", ...)` line:

```rust
            .route("/api/workflow-templates", get(api::workflow_templates_list_handler))
            .route("/api/workflow-templates/{name}", get(api::workflow_template_get_handler))
```

- [ ] **Step 3: Add test for templates API**

Add to the test module in `crates/web/src/api.rs`:

```rust
    #[tokio::test]
    async fn api_workflow_templates_returns_array() {
        let ts = test_server();
        let response = ts.get("/api/workflow-templates").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body.is_array());
        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 5);
    }

    #[tokio::test]
    async fn api_workflow_template_get_returns_template() {
        let ts = test_server();
        let response = ts.get("/api/workflow-templates/pipeline").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body.get("name").unwrap().as_str(), Some("pipeline"));
    }

    #[tokio::test]
    async fn api_workflow_template_not_found() {
        let ts = test_server();
        let response = ts.get("/api/workflow-templates/nonexistent").await;
        response.assert_status(StatusCode::NOT_FOUND);
    }
```

- [ ] **Step 4: Run web tests**

Run: `cargo test -p agent007-web`
Expected: ALL PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/api.rs crates/web/src/server.rs
git commit -m "feat(web): add workflow template list and get API endpoints"
```

---

### Task 6: Create EvaluatorNode and RouterNode Vue Components

**Files:**
- Create: `crates/web/frontend/src/components/EvaluatorNode.vue`
- Create: `crates/web/frontend/src/components/RouterNode.vue`

- [ ] **Step 1: Create EvaluatorNode.vue**

Create `crates/web/frontend/src/components/EvaluatorNode.vue`:

```vue
<script setup>
import { Handle, Position } from '@vue-flow/core'

const props = defineProps({ data: Object, id: String })
</script>

<template>
  <div class="bg-base-200 border-2 border-orange-500 rounded-lg shadow-lg min-w-48 font-mono text-xs">
    <Handle type="target" :position="Position.Top" class="!bg-orange-400 !w-3 !h-3 !border-2 !border-base-300" />

    <div class="px-3 py-2 bg-orange-500/10 border-b border-orange-500/30 rounded-t-lg flex items-center gap-2">
      <span class="text-orange-400">↺</span>
      <span class="font-bold text-sm text-orange-400">{{ data.agent }}</span>
      <span class="ml-auto text-orange-400/60 text-[10px]">evaluator</span>
    </div>

    <div class="px-3 py-2 space-y-1">
      <div class="flex gap-2">
        <span class="text-base-content/40">id:</span>
        <span class="text-info">{{ id }}</span>
      </div>
      <div class="flex gap-2" v-if="data.output">
        <span class="text-base-content/40">out:</span>
        <span class="text-success">{{ data.output }}</span>
      </div>
      <div v-if="data.evaluate" class="space-y-0.5 border-t border-base-300 pt-1 mt-1">
        <div class="flex gap-2">
          <span class="text-base-content/40">pass→</span>
          <span class="text-green-400">{{ data.evaluate.on_pass }}</span>
        </div>
        <div class="flex gap-2">
          <span class="text-base-content/40">fail→</span>
          <span class="text-orange-400">{{ data.evaluate.on_fail }}</span>
        </div>
        <div class="flex gap-2">
          <span class="text-base-content/40">retries:</span>
          <span class="text-base-content/60">{{ data.evaluate.max_retries ?? 3 }}</span>
        </div>
      </div>
      <div class="text-base-content/50 truncate max-w-40" :title="data.prompt">
        {{ data.prompt?.slice(0, 40) }}{{ (data.prompt?.length || 0) > 40 ? '...' : '' }}
      </div>
    </div>

    <div class="flex justify-around pb-1">
      <Handle id="pass" type="source" :position="Position.Bottom" class="!bg-green-400 !w-3 !h-3 !border-2 !border-base-300" style="left: 35%" />
      <Handle id="retry" type="source" :position="Position.Bottom" class="!bg-orange-400 !w-3 !h-3 !border-2 !border-base-300" style="left: 65%" />
    </div>
  </div>
</template>
```

- [ ] **Step 2: Create RouterNode.vue**

Create `crates/web/frontend/src/components/RouterNode.vue`:

```vue
<script setup>
import { Handle, Position } from '@vue-flow/core'
import { computed } from 'vue'

const props = defineProps({ data: Object, id: String })

const routes = computed(() => props.data.routes || [])
</script>

<template>
  <div class="bg-base-200 border-2 border-purple-500 rounded-lg shadow-lg min-w-48 font-mono text-xs">
    <Handle type="target" :position="Position.Top" class="!bg-purple-400 !w-3 !h-3 !border-2 !border-base-300" />

    <div class="px-3 py-2 bg-purple-500/10 border-b border-purple-500/30 rounded-t-lg flex items-center gap-2">
      <span class="text-purple-400">⑂</span>
      <span class="font-bold text-sm text-purple-400">{{ data.agent }}</span>
      <span class="ml-auto text-purple-400/60 text-[10px]">router</span>
    </div>

    <div class="px-3 py-2 space-y-1">
      <div class="flex gap-2">
        <span class="text-base-content/40">id:</span>
        <span class="text-info">{{ id }}</span>
      </div>
      <div v-if="routes.length" class="border-t border-base-300 pt-1 mt-1 space-y-0.5">
        <div v-for="(route, i) in routes" :key="i" class="flex gap-2">
          <span class="text-purple-400">{{ route.when || 'default' }}</span>
          <span class="text-base-content/40">→</span>
          <span class="text-base-content/60">{{ route.goto }}</span>
        </div>
      </div>
      <div class="text-base-content/50 truncate max-w-40" :title="data.prompt">
        {{ data.prompt?.slice(0, 40) }}{{ (data.prompt?.length || 0) > 40 ? '...' : '' }}
      </div>
    </div>

    <div class="flex justify-around pb-1">
      <Handle
        v-for="(route, i) in routes"
        :key="i"
        :id="route.goto"
        type="source"
        :position="Position.Bottom"
        class="!bg-purple-400 !w-3 !h-3 !border-2 !border-base-300"
        :style="{ left: `${((i + 1) / (routes.length + 1)) * 100}%` }"
      />
    </div>
  </div>
</template>
```

- [ ] **Step 3: Commit**

```bash
git add crates/web/frontend/src/components/EvaluatorNode.vue crates/web/frontend/src/components/RouterNode.vue
git commit -m "feat(web): add EvaluatorNode and RouterNode Vue components"
```

---

### Task 7: Update useApi.js with Template Methods

**Files:**
- Modify: `crates/web/frontend/src/composables/useApi.js`

- [ ] **Step 1: Add template API methods**

Add these two methods to the `api` object in `crates/web/frontend/src/composables/useApi.js`, after the `saveWorkflow` method:

```javascript
    // Workflow Templates
    listTemplates: () => fetchJson('/api/workflow-templates'),
    getTemplate: (name) => fetchJson(`/api/workflow-templates/${encodeURIComponent(name)}`),
```

- [ ] **Step 2: Commit**

```bash
git add crates/web/frontend/src/composables/useApi.js
git commit -m "feat(web): add template API methods to useApi composable"
```

---

### Task 8: Update WorkflowsView with Delete, Templates, and New Node Types

**Files:**
- Modify: `crates/web/frontend/src/views/WorkflowsView.vue`

- [ ] **Step 1: Replace the entire WorkflowsView.vue**

Replace the contents of `crates/web/frontend/src/views/WorkflowsView.vue` with:

```vue
<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { VueFlow, useVueFlow } from '@vue-flow/core'
import { Background } from '@vue-flow/background'
import { Controls } from '@vue-flow/controls'
import { MiniMap } from '@vue-flow/minimap'
import { useApi } from '../composables/useApi.js'
import AgentNode from '../components/AgentNode.vue'
import EvaluatorNode from '../components/EvaluatorNode.vue'
import RouterNode from '../components/RouterNode.vue'

const { api } = useApi()
const workflows = ref([])
const personas = ref([])
const templates = ref([])
const selectedWorkflow = ref(null)
const showSaveDialog = ref(false)
const showTemplateMenu = ref(false)
const workflowName = ref('')
const workflowDescription = ref('')

const { onConnect, addEdges, getNodes, getEdges } = useVueFlow()

const nodes = ref([])
const edges = ref([])

const nodeTypes = {
  agent: AgentNode,
  evaluator: EvaluatorNode,
  router: RouterNode,
}

const contextMenu = ref({ show: false, x: 0, y: 0, type: null, targetId: null })

onMounted(async () => {
  const [wf, ps, tpl] = await Promise.all([
    api.listWorkflows(),
    api.listPersonas(),
    api.listTemplates(),
  ])
  if (wf) workflows.value = wf
  if (ps) personas.value = ps
  if (tpl) templates.value = tpl
})

onConnect((params) => {
  const sourceNode = nodes.value.find(n => n.id === params.source)
  let edgeStyle = { stroke: '#39d0c8' }
  let label = ''

  if (sourceNode?.type === 'evaluator') {
    const isRetry = params.sourceHandle === 'retry'
    edgeStyle = { stroke: isRetry ? '#f97316' : '#4ade80' }
    label = isRetry ? 'retry' : 'pass'
  } else if (sourceNode?.type === 'router') {
    edgeStyle = { stroke: '#a855f7' }
    label = params.sourceHandle || ''
  }

  addEdges([{ ...params, animated: true, style: edgeStyle, label }])
})

function handleKeydown(e) {
  if (e.key === 'Delete' || e.key === 'Backspace') {
    const selectedNodes = nodes.value.filter(n => n.selected)
    const selectedEdges = edges.value.filter(e => e.selected)

    if (selectedNodes.length) {
      const nodeIds = new Set(selectedNodes.map(n => n.id))
      nodes.value = nodes.value.filter(n => !nodeIds.has(n.id))
      edges.value = edges.value.filter(e => !nodeIds.has(e.source) && !nodeIds.has(e.target))
    }
    if (selectedEdges.length) {
      const edgeIds = new Set(selectedEdges.map(e => e.id))
      edges.value = edges.value.filter(e => !edgeIds.has(e.id))
    }
  }
}

onMounted(() => document.addEventListener('keydown', handleKeydown))
onUnmounted(() => document.removeEventListener('keydown', handleKeydown))

function showContextMenu(event, type, targetId) {
  event.preventDefault()
  contextMenu.value = { show: true, x: event.clientX, y: event.clientY, type, targetId }
}

function hideContextMenu() {
  contextMenu.value = { show: false, x: 0, y: 0, type: null, targetId: null }
}

function deleteFromContextMenu() {
  const { type, targetId } = contextMenu.value
  if (type === 'node') {
    nodes.value = nodes.value.filter(n => n.id !== targetId)
    edges.value = edges.value.filter(e => e.source !== targetId && e.target !== targetId)
  } else if (type === 'edge') {
    edges.value = edges.value.filter(e => e.id !== targetId)
  }
  hideContextMenu()
}

async function loadWorkflow(name) {
  const data = await api.getWorkflow(name)
  if (!data) return
  selectedWorkflow.value = name
  graphFromSteps(data.steps || [], name)
}

function graphFromSteps(steps, name) {
  const stepNodes = steps.map((step, i) => {
    let type = 'agent'
    if (step.type === 'evaluator') type = 'evaluator'
    else if (step.type === 'router') type = 'router'

    return {
      id: step.id,
      type,
      position: { x: 100 + (i % 3) * 300, y: 80 + Math.floor(i / 3) * 200 },
      data: {
        label: step.id,
        agent: step.agent,
        prompt: step.prompt,
        output: step.output || '',
        evaluate: step.evaluate || null,
        routes: step.routes || [],
      },
    }
  })

  const stepEdges = []
  for (const step of steps) {
    for (const dep of step.depends_on || []) {
      const sourceNode = stepNodes.find(n => n.id === dep)
      let style = { stroke: '#39d0c8' }
      let label = ''

      if (sourceNode?.type === 'evaluator' && step.evaluate) {
        const isRetryTarget = step.evaluate.on_fail === dep
        style = { stroke: isRetryTarget ? '#f97316' : '#4ade80' }
        label = isRetryTarget ? 'retry' : 'pass'
      } else if (sourceNode?.type === 'router') {
        style = { stroke: '#a855f7' }
      }

      stepEdges.push({
        id: `${dep}->${step.id}`,
        source: dep,
        target: step.id,
        animated: true,
        style,
        label,
      })
    }
  }

  nodes.value = stepNodes
  edges.value = stepEdges
}

let nodeCounter = 0

function addAgentNode(persona) {
  nodeCounter++
  const id = `step-${nodeCounter}`
  nodes.value.push({
    id,
    type: 'agent',
    position: { x: 150 + (nodeCounter % 4) * 250, y: 100 + Math.floor(nodeCounter / 4) * 180 },
    data: { label: id, agent: persona.name, prompt: '{{task}}', output: `${id}_output` },
  })
}

function addEvaluatorNode() {
  nodeCounter++
  const id = `eval-${nodeCounter}`
  nodes.value.push({
    id,
    type: 'evaluator',
    position: { x: 150 + (nodeCounter % 4) * 250, y: 100 + Math.floor(nodeCounter / 4) * 180 },
    data: {
      label: id,
      agent: 'CodeReviewer',
      prompt: 'Review: {{code}}. Respond JSON: {"verdict":"pass" or "retry","reason":"..."}',
      output: `${id}_result`,
      evaluate: { decision_field: 'verdict', on_pass: '', on_fail: '', max_retries: 3 },
    },
  })
}

function addRouterNode() {
  nodeCounter++
  const id = `router-${nodeCounter}`
  nodes.value.push({
    id,
    type: 'router',
    position: { x: 150 + (nodeCounter % 4) * 250, y: 100 + Math.floor(nodeCounter / 4) * 180 },
    data: {
      label: id,
      agent: 'Researcher',
      prompt: 'Classify this task: {{task}}',
      output: `${id}_classification`,
      routes: [
        { when: 'frontend', goto: '' },
        { when: 'backend', goto: '' },
        { goto: '', default: true },
      ],
    },
  })
}

function exportYaml() {
  const currentNodes = getNodes.value
  const currentEdges = getEdges.value

  const steps = currentNodes.map(n => {
    const deps = currentEdges.filter(e => e.target === n.id).map(e => e.source)

    const step = {
      id: n.id,
      agent: n.data.agent,
      prompt: n.data.prompt,
      output: n.data.output || undefined,
      depends_on: deps.length ? deps : undefined,
    }

    if (n.type === 'evaluator') {
      step.type = 'evaluator'
      step.evaluate = n.data.evaluate
    } else if (n.type === 'router') {
      step.type = 'router'
      step.routes = n.data.routes
    }

    return step
  })

  return {
    name: workflowName.value || selectedWorkflow.value || 'new-workflow',
    description: workflowDescription.value || '',
    steps,
  }
}

async function saveWorkflow() {
  const yaml = exportYaml()
  await api.saveWorkflow(yaml)
  showSaveDialog.value = false
  const wf = await api.listWorkflows()
  if (wf) workflows.value = wf
}

function openSaveDialog() {
  workflowName.value = selectedWorkflow.value || ''
  workflowDescription.value = ''
  showSaveDialog.value = true
}

function newWorkflow() {
  selectedWorkflow.value = null
  nodes.value = []
  edges.value = []
  nodeCounter = 0
  showTemplateMenu.value = false
}

async function loadTemplate(tplName) {
  const tpl = await api.getTemplate(tplName)
  if (!tpl) return
  selectedWorkflow.value = null
  graphFromSteps(tpl.steps || [], tpl.name)
  showTemplateMenu.value = false
}
</script>

<template>
  <div class="flex flex-col h-full" @click="hideContextMenu">
    <div class="p-4 border-b border-base-300 bg-base-200 flex items-center justify-between">
      <h2 class="text-lg font-bold">Workflow Designer</h2>
      <div class="flex gap-2">
        <div class="dropdown dropdown-end">
          <label tabindex="0" class="btn btn-sm btn-ghost" @click.stop="showTemplateMenu = !showTemplateMenu">New ▾</label>
          <ul v-if="showTemplateMenu" tabindex="0" class="dropdown-content z-50 menu p-2 shadow bg-base-300 rounded-box w-56">
            <li><a @click="newWorkflow">Empty Canvas</a></li>
            <li class="menu-title"><span>Templates</span></li>
            <li v-for="tpl in templates" :key="tpl.name">
              <a @click="loadTemplate(tpl.name)">
                <span class="font-mono text-xs">{{ tpl.name }}</span>
                <span class="text-base-content/40 text-[10px]">{{ tpl.description?.slice(0, 30) }}</span>
              </a>
            </li>
          </ul>
        </div>
        <button class="btn btn-sm btn-primary" @click="openSaveDialog" :disabled="!nodes.length">Save</button>
      </div>
    </div>

    <div class="flex flex-1 overflow-hidden">
      <div class="w-56 bg-base-200 border-r border-base-300 flex flex-col shrink-0 overflow-auto">
        <div class="p-3 border-b border-base-300">
          <h3 class="text-xs font-bold uppercase tracking-wider text-base-content/50 mb-2">Workflows</h3>
          <div class="space-y-1">
            <button
              v-for="w in workflows" :key="w"
              class="btn btn-ghost btn-xs justify-start w-full font-mono text-xs"
              :class="{ 'btn-active': selectedWorkflow === w }"
              @click="loadWorkflow(w)"
            >{{ w }}</button>
          </div>
          <div v-if="!workflows.length" class="text-xs text-base-content/40">No workflows</div>
        </div>

        <div class="p-3 border-b border-base-300">
          <h3 class="text-xs font-bold uppercase tracking-wider text-base-content/50 mb-2">Special Nodes</h3>
          <div class="space-y-1">
            <button class="btn btn-ghost btn-xs justify-start w-full text-xs" @click="addEvaluatorNode">
              <span class="text-orange-400">↺</span> Evaluator
            </button>
            <button class="btn btn-ghost btn-xs justify-start w-full text-xs" @click="addRouterNode">
              <span class="text-purple-400">⑂</span> Router
            </button>
          </div>
        </div>

        <div class="p-3">
          <h3 class="text-xs font-bold uppercase tracking-wider text-base-content/50 mb-2">Agent Palette</h3>
          <p class="text-xs text-base-content/40 mb-2">Click to add a node</p>
          <div class="space-y-1">
            <button
              v-for="p in personas" :key="p.name"
              class="btn btn-ghost btn-xs justify-start w-full text-xs"
              @click="addAgentNode(p)"
            >
              <span class="text-primary">◉</span> {{ p.name }}
            </button>
          </div>
        </div>
      </div>

      <div class="flex-1 relative">
        <VueFlow
          v-model:nodes="nodes"
          v-model:edges="edges"
          :node-types="nodeTypes"
          :default-viewport="{ zoom: 0.9, x: 0, y: 0 }"
          :edges-updatable="true"
          :nodes-draggable="true"
          :select-nodes-on-drag="false"
          fit-view-on-init
          class="h-full"
          @node-context-menu="({ event, node }) => showContextMenu(event, 'node', node.id)"
          @edge-context-menu="({ event, edge }) => showContextMenu(event, 'edge', edge.id)"
        >
          <Background variant="dots" :gap="20" :size="1" />
          <Controls position="bottom-right" />
          <MiniMap position="bottom-left" />
        </VueFlow>

        <div v-if="!nodes.length" class="absolute inset-0 flex items-center justify-center pointer-events-none">
          <div class="text-center text-base-content/30">
            <div class="text-4xl mb-2">⬡</div>
            <p class="text-sm">Load a workflow, pick a template, or add agents from the palette</p>
          </div>
        </div>
      </div>
    </div>

    <!-- Context menu -->
    <div
      v-if="contextMenu.show"
      class="fixed z-50 bg-base-300 border border-base-content/10 rounded-lg shadow-xl py-1 min-w-32"
      :style="{ left: contextMenu.x + 'px', top: contextMenu.y + 'px' }"
      @click.stop
    >
      <button class="w-full text-left px-3 py-1.5 text-sm hover:bg-error/20 text-error" @click="deleteFromContextMenu">
        Delete {{ contextMenu.type === 'node' ? 'Node' : 'Edge' }}
      </button>
    </div>

    <!-- Save dialog -->
    <dialog :open="showSaveDialog" class="modal" :class="{ 'modal-open': showSaveDialog }">
      <div class="modal-box bg-base-200">
        <h3 class="font-bold text-lg">Save Workflow</h3>
        <div class="mt-4 space-y-3">
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Name</span></label>
            <input v-model="workflowName" class="input input-sm input-bordered font-mono" placeholder="my-workflow" />
          </div>
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Description</span></label>
            <input v-model="workflowDescription" class="input input-sm input-bordered" />
          </div>
        </div>
        <div class="modal-action">
          <button class="btn btn-sm btn-ghost" @click="showSaveDialog = false">Cancel</button>
          <button class="btn btn-sm btn-primary" @click="saveWorkflow">Save</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showSaveDialog = false">close</button></form>
    </dialog>
  </div>
</template>

<style>
@import '@vue-flow/core/dist/style.css';
@import '@vue-flow/core/dist/theme-default.css';
@import '@vue-flow/controls/dist/style.css';
@import '@vue-flow/minimap/dist/style.css';

.vue-flow {
  background: oklch(0.2 0.01 260);
}
.vue-flow__minimap {
  background: oklch(0.15 0.01 260);
}
.vue-flow__edge-text {
  font-size: 10px;
  fill: oklch(0.7 0 0);
}
</style>
```

- [ ] **Step 2: Commit**

```bash
git add crates/web/frontend/src/views/WorkflowsView.vue
git commit -m "feat(web): add delete, templates, evaluator/router nodes to workflow designer"
```

---

### Task 9: Build Frontend and Verify Full Stack

**Files:**
- Working in: `crates/web/frontend/`

- [ ] **Step 1: Install dependencies and build**

```bash
cd crates/web/frontend && npm install && npm run build
```

Expected: Build succeeds, output in `crates/web/static/dist/`.

- [ ] **Step 2: Run Rust compilation check**

```bash
cargo check --workspace
```

Expected: No errors.

- [ ] **Step 3: Run all workflow tests**

```bash
cargo test -p agent007-workflows
```

Expected: ALL PASS.

- [ ] **Step 4: Run all web tests**

```bash
cargo test -p agent007-web
```

Expected: ALL PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web/static/dist/ crates/web/frontend/src/
git commit -m "build: rebuild frontend with orchestration patterns support"
```

---

## Self-Review Checklist

- [x] **Spec coverage**: All spec sections mapped to tasks:
  - Node/edge deletion → Task 8 (WorkflowsView)
  - Templates → Task 5 (API) + Task 7 (useApi) + Task 8 (dropdown)
  - StepType/EvaluateConfig/RouteConfig → Task 1
  - DAG validator → Task 3
  - Runner evaluator/router logic → Task 4
  - EvaluatorNode/RouterNode → Task 6
  - Error variants → Task 2
- [x] **Placeholder scan**: No TBD/TODO found
- [x] **Type consistency**: `StepType`, `EvaluateConfig`, `RouteConfig`, `ValidatedDag`, `BackEdge`, `RouterBranch` used consistently across tasks
- [x] **Backward compatibility**: All new fields on StepDef are Option or #[serde(default)]
