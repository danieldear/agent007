use anyhow::Result;
use serde_json::{json, Value};

use super::graph_common::open_index_maybe_build;

pub fn run(input: &Value) -> Result<Value> {
    let query = input["query"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("query required"))?;
    let max_symbols = input["max_symbols"].as_u64().unwrap_or(5) as usize;
    let max_neighbors = input["max_neighbors"].as_u64().unwrap_or(3) as usize;
    let (_, index_path, index) = open_index_maybe_build(input)?;
    let out = index.context_bundle_for_query(query, max_symbols, max_neighbors)?;
    Ok(json!({
        "query": out.query,
        "symbol_count": out.matched_symbols.len(),
        "doc_count": out.related_docs.len(),
        "file_count": out.files.len(),
        "matched_symbols": out.matched_symbols,
        "related_docs": out.related_docs,
        "files": out.files,
        "text": out.text,
        "source": "repo_index_v2",
        "index_path": index_path.display().to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_context_bundle_from_graph() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        let out = run(&json!({
            "root": dir.path(),
            "query": "alpha behavior",
            "build_if_missing": true
        }))
        .unwrap();
        assert!(out["symbol_count"].as_u64().unwrap_or(0) >= 1);
        assert!(out["text"].as_str().unwrap_or("").contains("alpha"));
    }
}
