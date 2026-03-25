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
            make_step("b", &[], &["a"], None), // no artifact dep, just ordering
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
