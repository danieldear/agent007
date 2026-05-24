use anyhow::Result;
use serde_json::Value;

use super::graph_common::load_graph_maybe_build;

pub fn run(input: &Value) -> Result<Value> {
    let from_symbol = input["from_symbol"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("from_symbol required"))?;
    let to_symbol = input["to_symbol"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("to_symbol required"))?;
    let exact = input["exact"].as_bool().unwrap_or(true);
    let (_, graph) = load_graph_maybe_build(input)?;
    let path = agent007_core::dep_path_between_symbols(&graph, from_symbol, to_symbol, exact);
    Ok(serde_json::to_value(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn returns_dependency_path_between_symbols() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\npub fn gamma() { beta(); }\n",
        )
        .unwrap();
        let out = run(&json!({
            "root": dir.path(),
            "from_symbol": "gamma",
            "to_symbol": "alpha",
            "build_if_missing": true
        }))
        .unwrap();
        assert_eq!(out["found"], true);
        assert_eq!(out["steps"].as_array().map(|v| v.len()).unwrap_or(0), 2);
    }
}
