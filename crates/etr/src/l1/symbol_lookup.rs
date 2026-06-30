use anyhow::Result;
use serde_json::{json, Value};

use super::graph_common::open_index_maybe_build;

pub fn run(input: &Value) -> Result<Value> {
    let symbol = input["symbol"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("symbol required"))?;
    let exact = input["exact"].as_bool().unwrap_or(false);
    let (_, index_path, index) = open_index_maybe_build(input)?;
    let matches = index.symbol_lookup(symbol, exact)?;
    Ok(json!({
        "symbol": symbol,
        "exact": exact,
        "count": matches.len(),
        "matches": matches,
        "source": "repo_index_v2",
        "index_path": index_path.display().to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_symbols_after_build() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}").unwrap();
        let out =
            run(&json!({"root": dir.path(), "symbol": "alpha", "build_if_missing": true})).unwrap();
        assert!(out["count"].as_u64().unwrap_or(0) >= 1);
    }

    #[test]
    fn build_if_missing_creates_index_without_legacy_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}").unwrap();

        let out = run(&json!({
            "root": dir.path(),
            "symbol": "alpha",
            "exact": true,
            "build_if_missing": true
        }))
        .unwrap();

        assert_eq!(out["count"].as_u64().unwrap_or(0), 1);
        assert!(agent007_core::default_index_path_for_root(dir.path()).exists());
        assert!(!agent007_core::default_graph_path_for_root(dir.path()).exists());
    }

    #[test]
    fn auto_refreshes_dirty_graph_before_lookup() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}").unwrap();
        let graph_path = agent007_core::default_graph_path_for_root(dir.path());
        agent007_core::build_and_save_graph(dir.path(), Some(&graph_path)).unwrap();

        std::fs::write(dir.path().join("src/lib.rs"), "pub fn beta() {}").unwrap();
        agent007_core::mark_repo_graph_dirty_paths(
            dir.path(),
            &[std::path::PathBuf::from("src/lib.rs")],
        )
        .unwrap();

        let out = run(&json!({
            "root": dir.path(),
            "symbol": "beta",
            "exact": true,
            "build_if_missing": true
        }))
        .unwrap();
        assert_eq!(out["count"].as_u64().unwrap_or(0), 1);
        assert!(!agent007_core::graph_status(&graph_path).stale);
    }
}
