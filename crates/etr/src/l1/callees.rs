use anyhow::Result;
use serde_json::{json, Value};

use super::graph_common::open_index_maybe_build;

pub fn run(input: &Value) -> Result<Value> {
    let symbol = input["symbol"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("symbol required"))?;
    let exact = input["exact"].as_bool().unwrap_or(true);
    let (_, index_path, index) = open_index_maybe_build(input)?;
    let rows = index.callees_for_symbol(symbol, exact)?;
    Ok(
        json!({"symbol": symbol, "exact": exact, "count": rows.len(), "callees": rows, "source": "repo_index_v2", "index_path": index_path.display().to_string()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_callees_for_symbol() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        let out =
            run(&json!({"root": dir.path(), "symbol": "beta", "build_if_missing": true})).unwrap();
        assert!(out["count"].as_u64().unwrap_or(0) >= 1);
    }
}
