use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub fn run(input: &Value) -> Result<Value> {
    let left_key = input["left_key"].as_str().context("left_key required")?;
    let right_key = input["right_key"].as_str().context("right_key required")?;
    let how = input["how"].as_str().unwrap_or("inner");

    let left = load_side(input, "left_rows", "left_path", "left_format")?;
    let right = load_side(input, "right_rows", "right_path", "right_format")?;

    let mut right_index: HashMap<String, Vec<Map<String, Value>>> = HashMap::new();
    for r in right {
        let key = r.get(right_key).cloned().unwrap_or(Value::Null).to_string();
        right_index.entry(key).or_default().push(r);
    }

    let mut out = Vec::new();
    for l in left {
        let key = l.get(left_key).cloned().unwrap_or(Value::Null).to_string();
        if let Some(rr) = right_index.get(&key) {
            for r in rr {
                let mut merged = l.clone();
                for (k, v) in r {
                    merged.insert(format!("right.{k}"), v.clone());
                }
                out.push(Value::Object(merged));
            }
        } else if how == "left" {
            out.push(Value::Object(l.clone()));
        }
    }
    Ok(json!({"rows": out, "count": out.len()}))
}

fn load_side(
    input: &Value,
    rows_key: &str,
    path_key: &str,
    format_key: &str,
) -> Result<Vec<Map<String, Value>>> {
    if let Some(rows) = input.get(rows_key).and_then(Value::as_array) {
        return Ok(rows
            .iter()
            .filter_map(|v| v.as_object().cloned())
            .collect::<Vec<_>>());
    }
    let path = input[path_key].as_str().context("path required")?;
    let format = input.get(format_key).cloned().unwrap_or(Value::String("auto".into()));
    let rows = super::table_select::run(&json!({"path":path,"format":format}))?["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(rows
        .into_iter()
        .filter_map(|v| v.as_object().cloned())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins_rows() {
        let out = run(&json!({
            "left_key":"id","right_key":"id",
            "left_rows":[{"id":"1","a":"x"}],
            "right_rows":[{"id":"1","b":"y"}]
        }))
        .unwrap();
        assert_eq!(out["count"], 1);
    }
}

