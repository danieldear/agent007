use anyhow::Result;
use serde_json::{json, Value};

use super::graph_common::load_graph_maybe_build;

pub fn run(input: &Value) -> Result<Value> {
    let symbol = input["symbol"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("symbol required"))?;
    let exact = input["exact"].as_bool().unwrap_or(true);
    let (_, graph) = load_graph_maybe_build(input)?;
    let rows = agent007_core::doc_links_for_symbol(&graph, symbol, exact);
    Ok(json!({"symbol": symbol, "exact": exact, "count": rows.len(), "docs": rows}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_docs_for_symbol() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(
            dir.path().join("docs/alpha.md"),
            "Documentation for `alpha`.\n",
        )
        .unwrap();
        let out =
            run(&json!({"root": dir.path(), "symbol": "alpha", "build_if_missing": true})).unwrap();
        assert!(out["count"].as_u64().unwrap_or(0) >= 1);
    }
}
