use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn run(input: &Value) -> Result<Value> {
    let field = input["field"].as_str().context("field required")?;
    let top_k = input["top_k"].as_u64().unwrap_or(50) as usize;
    let rows = if let Some(path) = input["path"].as_str() {
        super::table_select::run(&json!({"path":path,"format":input["format"]}))
            .ok()
            .and_then(|v| v["rows"].as_array().cloned())
            .unwrap_or_default()
    } else {
        input["rows"].as_array().cloned().unwrap_or_default()
    };

    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let key = row
            .get(field)
            .cloned()
            .unwrap_or(Value::Null)
            .to_string()
            .trim_matches('"')
            .to_string();
        *counts.entry(key).or_insert(0) += 1;
    }
    let mut pairs = counts
        .into_iter()
        .map(|(k, v)| json!({"key": k, "count": v}))
        .collect::<Vec<_>>();
    pairs.sort_by(|a, b| {
        b["count"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["count"].as_u64().unwrap_or(0))
    });
    pairs.truncate(top_k);
    Ok(json!({"groups": pairs, "count": pairs.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_groups() {
        let out = run(&json!({"field":"k","rows":[{"k":"a"},{"k":"a"},{"k":"b"}]})).unwrap();
        assert_eq!(out["groups"][0]["key"], "a");
        assert_eq!(out["groups"][0]["count"], 2);
    }
}
