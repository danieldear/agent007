use crate::error::WorkflowError;
use crate::types::{RouteConfig, StepType, WorkflowDef};
use petgraph::algo::toposort;
use petgraph::graph::DiGraph;
use std::collections::HashMap;

#[derive(Debug)]
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

        let mut back_edges = Vec::new();
        let mut evaluator_back_edge_set: HashMap<(&str, &str), ()> = HashMap::new();

        for step in &self.def.steps {
            if step.r#type == StepType::Evaluator {
                let eval =
                    step.evaluate
                        .as_ref()
                        .ok_or_else(|| WorkflowError::InvalidEvaluator {
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
                evaluator_back_edge_set.insert((step.id.as_str(), eval.on_fail.as_str()), ());
            }
        }

        let mut router_branches = Vec::new();
        for step in &self.def.steps {
            if step.r#type == StepType::Router {
                let routes = step
                    .routes
                    .as_ref()
                    .ok_or_else(|| WorkflowError::InvalidRouter {
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

        let mut output_to_step: HashMap<String, String> = HashMap::new();
        for step in &self.def.steps {
            if let Some(out) = &step.output {
                output_to_step.insert(out.clone(), step.id.clone());
            }
        }

        let mut graph: DiGraph<String, ()> = DiGraph::new();
        let node_indices: Vec<_> = self
            .def
            .steps
            .iter()
            .map(|s| graph.add_node(s.id.clone()))
            .collect();
        let id_to_node: HashMap<String, _> = self
            .def
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), node_indices[i]))
            .collect();

        // Built-in input variables injected at runtime — not produced by any step.
        const BUILTIN_INPUTS: &[&str] = &[
            "memory.project",
            "memory.user",
            "memory.global",
            "rag_context",
            "task",
        ];

        for step in &self.def.steps {
            let to_node = id_to_node[&step.id];

            for inp in step.inputs.iter().flatten() {
                if BUILTIN_INPUTS.contains(&inp.as_str()) {
                    continue;
                }
                let producer =
                    output_to_step
                        .get(inp)
                        .ok_or_else(|| WorkflowError::UnknownInput {
                            id: step.id.clone(),
                            input: inp.clone(),
                        })?;

                if evaluator_back_edge_set.contains_key(&(step.id.as_str(), producer.as_str())) {
                    continue;
                }

                let from_node = id_to_node[producer];
                graph.add_edge(from_node, to_node, ());
            }

            for dep in step.depends_on.iter().flatten() {
                let from_node = id_to_node
                    .get(dep)
                    .ok_or_else(|| WorkflowError::UnknownInput {
                        id: step.id.clone(),
                        input: dep.clone(),
                    })?;

                if evaluator_back_edge_set.contains_key(&(step.id.as_str(), dep.as_str())) {
                    continue;
                }

                graph.add_edge(*from_node, to_node, ());
            }
        }

        let topo_order = toposort(&graph, None).map_err(|_| WorkflowError::CycleDetected)?;

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
    use crate::types::{EvaluateConfig, RouteConfig, StepDef, StepType, WorkflowDef};

    fn make_step(id: &str, inputs: &[&str], depends_on: &[&str], output: Option<&str>) -> StepDef {
        StepDef {
            id: id.to_string(),
            agent: "TestAgent".to_string(),
            model: None,
            inputs: if inputs.is_empty() {
                None
            } else {
                Some(inputs.iter().map(|s| s.to_string()).collect())
            },
            depends_on: if depends_on.is_empty() {
                None
            } else {
                Some(depends_on.iter().map(|s| s.to_string()).collect())
            },
            prompt: Some("do {{task}}".to_string()),
            skill: None,
            output: output.map(|s| s.to_string()),
            requires_approval: None,
            r#type: StepType::Execute,
            evaluate: None,
            routes: None,
            workflow: None,
        }
    }

    fn make_def(steps: Vec<StepDef>) -> WorkflowDef {
        WorkflowDef {
            name: "test".to_string(),
            description: None,
            steps,
            budget: None,
        }
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
        let def = make_def(vec![make_step("a", &["nonexistent_output"], &[], None)]);
        let err = DagValidator::new(&def).validate().unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::UnknownInput { .. }
        ));
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
        assert!(
            result.is_ok(),
            "evaluator back-edge should not be detected as a cycle"
        );
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
            RouteConfig {
                when: Some("frontend".to_string()),
                goto: "ui".to_string(),
                default: false,
            },
            RouteConfig {
                when: None,
                goto: "api".to_string(),
                default: true,
            },
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
        router_step.routes = Some(vec![RouteConfig {
            when: Some("x".to_string()),
            goto: "nonexistent".to_string(),
            default: false,
        }]);

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

    #[test]
    fn builtin_inputs_do_not_trigger_unknown_input_error() {
        let def = make_def(vec![make_step(
            "a",
            &[
                "memory.project",
                "memory.user",
                "memory.global",
                "rag_context",
                "task",
            ],
            &[],
            None,
        )]);
        let result = DagValidator::new(&def).validate();
        assert!(
            result.is_ok(),
            "builtin inputs should not fail validation: {:?}",
            result.err()
        );
    }

    #[test]
    fn non_builtin_unknown_input_still_fails() {
        let def = make_def(vec![make_step(
            "a",
            &["definitely_unknown_output"],
            &[],
            None,
        )]);
        let err = DagValidator::new(&def).validate().unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::UnknownInput { .. }
        ));
    }
}
