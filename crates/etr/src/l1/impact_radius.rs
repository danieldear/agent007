use anyhow::Result;
use serde_json::Value;

use super::graph_common::open_index_maybe_build;

pub fn run(input: &Value) -> Result<Value> {
    let symbol = input["symbol"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("symbol required"))?;
    let exact = input["exact"].as_bool().unwrap_or(true);
    let max_depth = input["max_depth"].as_u64().unwrap_or(2) as usize;
    let (_, _index_path, index) = open_index_maybe_build(input)?;
    let out = index.usage_graph_for_symbol(symbol, exact, max_depth.max(2))?;
    Ok(serde_json::to_value(out)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expands_impact_radius() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\npub fn gamma() { beta(); }\n",
        )
        .unwrap();
        let out = run(&json!({
            "root": dir.path(),
            "symbol": "alpha",
            "build_if_missing": true,
            "max_depth": 2
        }))
        .unwrap();
        assert!(out["nodes"].as_array().map(|v| v.len()).unwrap_or(0) >= 2);
    }
}
