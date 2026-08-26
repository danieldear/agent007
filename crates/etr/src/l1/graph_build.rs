use anyhow::Result;
use serde_json::{json, Value};

use super::graph_common::{input_graph_path, input_root};

pub fn run(input: &Value) -> Result<Value> {
    let root = input_root(input)?;
    let graph_path = input_graph_path(input, &root);
    let index_path = agent007_core::index_path_for_graph_path(&graph_path);

    // `index_only` skips the legacy graph JSON, which is subject to a hard load
    // budget and is far larger on disk than the index built from the same walk.
    // Every graph query tool except `dep_path` reads the index.
    if input["index_only"].as_bool().unwrap_or(false) {
        let status = agent007_core::build_and_save_index(&root, Some(&index_path))?;
        let readiness = agent007_core::write_repo_intelligence_readiness(
            &root,
            None,
            &agent007_core::RepoIntelligenceOptions::default(),
        )?;
        return Ok(json!({
            "built": true,
            "index_only": true,
            "root": status.root,
            "index_path": status.index_path,
            "version": status.version,
            "counts": status.counts,
            "readiness": readiness,
        }));
    }

    let graph = agent007_core::build_and_save_graph(&root, Some(&graph_path))?;
    let readiness = agent007_core::write_repo_intelligence_readiness(
        &root,
        None,
        &agent007_core::RepoIntelligenceOptions::default(),
    )?;
    Ok(json!({
        "built": true,
        "index_only": false,
        "root": graph.root,
        "graph_path": graph.graph_path,
        "index_path": index_path.display().to_string(),
        "version": graph.version,
        "counts": graph.counts,
        "readiness": readiness,
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
        assert!(out["index_path"]
            .as_str()
            .unwrap()
            .contains("repo_index_v2.redb"));
    }
}
