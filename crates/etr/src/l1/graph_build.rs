use anyhow::Result;
use serde_json::{json, Value};

use super::graph_common::{input_graph_path, input_root};

pub fn run(input: &Value) -> Result<Value> {
    let root = input_root(input)?;
    let graph_path = input_graph_path(input, &root);
    let graph = agent007_core::build_and_save_graph(&root, Some(&graph_path))?;
    Ok(json!({
        "built": true,
        "root": graph.root,
        "graph_path": graph.graph_path,
        "version": graph.version,
        "counts": graph.counts,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_graph_and_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn hello() {}").unwrap();
        let out = run(&json!({"root": dir.path()})).unwrap();
        assert_eq!(out["built"], true);
        assert!(out["graph_path"]
            .as_str()
            .unwrap()
            .contains("repo_graph_v1.json"));
    }
}
