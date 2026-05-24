use anyhow::Result;
use serde_json::{json, Value};

use super::graph_common::{input_graph_path, input_root};

pub fn run(input: &Value) -> Result<Value> {
    let root = input_root(input)?;
    let graph_path = input_graph_path(input, &root);
    let paths: Vec<String> = input["paths"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let before = agent007_core::graph_status(&graph_path);
    let graph = agent007_core::build_and_save_graph(&root, Some(&graph_path))?;
    let after = agent007_core::graph_status(&graph_path);
    Ok(json!({
        "refreshed": true,
        "strategy": "full_rebuild_for_requested_paths",
        "requested_paths": paths,
        "graph_path": graph.graph_path,
        "root": graph.root,
        "version": graph.version,
        "counts": graph.counts,
        "before": before,
        "after": after,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_requested_paths_on_refresh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        let out = run(&json!({
            "root": dir.path(),
            "paths": ["src/lib.rs"]
        }))
        .unwrap();
        assert_eq!(out["refreshed"], true);
        assert_eq!(out["requested_paths"][0], "src/lib.rs");
    }
}
