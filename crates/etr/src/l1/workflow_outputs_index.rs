use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let text = std::fs::read_to_string(path)?;
    let root: Value = serde_json::from_str(&text)?;
    let outputs = root
        .pointer("/progress/outputs_available")
        .or_else(|| root.get("outputs_available"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let items = outputs
        .into_iter()
        .filter_map(|v| v.as_str().map(|s| json!({"key":s,"type":"output"})))
        .collect::<Vec<_>>();
    Ok(json!({"outputs": items, "count": items.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexes_outputs() {
        let p = std::env::temp_dir().join(format!("etr-wfo-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&p, r#"{"progress":{"outputs_available":["a","b"]}}"#).unwrap();
        let out = run(&json!({"path":p})).unwrap();
        assert_eq!(out["count"], 2);
    }
}

