use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let jq_path = input["jq_path"].as_str().context("jq_path required")?;

    let content =
        std::fs::read_to_string(path).context(format!("cannot read {path}"))?;
    let doc: Value = serde_json::from_str(&content).context("invalid JSON")?;

    let extracted = extract_path(&doc, jq_path)?;
    Ok(json!({ "value": extracted }))
}

fn extract_path<'a>(mut v: &'a Value, path: &str) -> Result<&'a Value> {
    let path = path.trim_start_matches('.');
    if path.is_empty() {
        return Ok(v);
    }
    for segment in path.split('.') {
        if let Ok(idx) = segment.parse::<usize>() {
            v = v.get(idx).context(format!("index {idx} out of range"))?;
        } else {
            v = v.get(segment).context(format!("key '{segment}' not found"))?;
        }
    }
    Ok(v)
}
