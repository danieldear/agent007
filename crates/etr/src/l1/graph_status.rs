use anyhow::Result;
use serde_json::{json, Value};

use super::graph_common::{input_graph_path, input_root};

pub fn run(input: &Value) -> Result<Value> {
    let root = input_root(input)?;
    let graph_path = input_graph_path(input, &root);
    let status = agent007_core::graph_status(&graph_path);
    let index_path = agent007_core::index_path_for_graph_path(&graph_path);
    let index = agent007_core::index_status(&index_path);
    Ok(json!({
        "exists": status.exists,
        "graph_path": status.graph_path,
        "root": status.root,
        "built_at": status.built_at,
        "version": status.version,
        "counts": status.counts,
        "stale": status.stale,
        "stale_files": status.stale_files,
        "missing_files": status.missing_files,
        "freshness": status.freshness,
        "dirty_paths": status.dirty_paths,
        "last_error": status.last_error,
        "index": index,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_missing_graph() {
        let dir = tempfile::tempdir().unwrap();
        let out = run(&json!({"root": dir.path()})).unwrap();
        assert_eq!(out["exists"], false);
    }
}
