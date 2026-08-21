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
    fn auto_refreshes_dirty_index_before_lookup() {
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
        // Finding `beta` at all proves the refresh ran. Freshness is now settled
        // against the index; the legacy graph is no longer on the query path.
        assert_eq!(out["count"].as_u64().unwrap_or(0), 1);
        let index = agent007_core::RepoIndex::open(
            &agent007_core::index_path_for_graph_path(&graph_path),
        )
        .unwrap();
        assert!(!agent007_core::index_is_stale(&index).unwrap());
    }

    #[test]
    fn lookup_ignores_oversized_legacy_graph_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}").unwrap();

        // Build the index first so the query can be served from it alone.
        run(&json!({"root": dir.path(), "symbol": "alpha", "build_if_missing": true})).unwrap();

        // Stand in for the real-world failure: a legacy graph JSON past the load
        // budget. It used to trigger a full rebuild on every single query that
        // could never bring the file back under budget.
        let graph_path = dir
            .path()
            .join(".agent007/runtime/repo_graph_v1.json");
        std::fs::create_dir_all(graph_path.parent().unwrap()).unwrap();
        let oversized = "x".repeat(1024);
        std::fs::write(&graph_path, &oversized).unwrap();

        let out = run(&json!({"root": dir.path(), "symbol": "alpha", "exact": true})).unwrap();
        assert_eq!(out["source"], "repo_index_v2");
        assert!(out["count"].as_u64().unwrap_or(0) >= 1, "index must still answer");
        assert_eq!(
            std::fs::read_to_string(&graph_path).unwrap(),
            oversized,
            "query path must not rewrite the legacy graph JSON"
        );
    }

    #[test]
    fn lookup_rebuilds_index_when_a_tracked_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}").unwrap();
        run(&json!({"root": dir.path(), "symbol": "alpha", "build_if_missing": true})).unwrap();

        // A symbol added after the index was built is only visible if the
        // freshness check actually rebuilds.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn added_later() {}",
        )
        .unwrap();

        let out = run(&json!({"root": dir.path(), "symbol": "added_later", "exact": true})).unwrap();
        assert!(
            out["count"].as_u64().unwrap_or(0) >= 1,
            "stale index must be refreshed without the legacy graph: {out}"
        );
    }
}
