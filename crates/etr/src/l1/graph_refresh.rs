use anyhow::Result;
use serde_json::{json, Value};

use super::graph_common::{input_graph_path, input_root};

pub fn run(input: &Value) -> Result<Value> {
    let root = input_root(input)?;
    let graph_path = input_graph_path(input, &root);
    let before = agent007_core::graph_status(&graph_path);
    let graph = agent007_core::build_and_save_graph(&root, Some(&graph_path))?;
    let after = agent007_core::graph_status(&graph_path);
    let readiness = agent007_core::write_repo_intelligence_readiness(
        &root,
        None,
        &agent007_core::RepoIntelligenceOptions::default(),
    )?;
    Ok(json!({
        "refreshed": true,
        "strategy": "full_rebuild",
        "graph_path": graph.graph_path,
        "index_path": agent007_core::index_path_for_graph_path(std::path::Path::new(&graph.graph_path)).display().to_string(),
        "root": graph.root,
        "version": graph.version,
        "counts": graph.counts,
        "before": before,
        "after": after,
        "readiness": readiness,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_existing_graph() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        let out = run(&json!({"root": dir.path()})).unwrap();
        assert_eq!(out["refreshed"], true);
        assert_eq!(out["strategy"], "full_rebuild");
    }
}
