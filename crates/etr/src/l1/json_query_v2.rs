use anyhow::{Context, Result};
use serde_json::{json, Value};

use super::json_query;

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let query = input["query"].as_str().context("query required")?;
    let sort_by = input["sort_by"].as_str();
    let sort_order = input["sort_order"].as_str().unwrap_or("asc");
    let limit = input["limit"].as_u64().map(|n| n as usize);
    let project = input["project"].as_array().cloned().unwrap_or_default();

    let base = json_query::run(&json!({ "path": path, "query": query }))?;
    let mut matches = base["matches"].as_array().cloned().unwrap_or_default();

    if !project.is_empty() {
        matches = matches
            .into_iter()
            .map(|m| project_value(m, &project))
            .collect::<Vec<_>>();
    }

    if let Some(key) = sort_by {
        matches.sort_by(|a, b| {
            let av = value_for_sort(a, key);
            let bv = value_for_sort(b, key);
            av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
        });
        if sort_order.eq_ignore_ascii_case("desc") {
            matches.reverse();
        }
    }

    if let Some(n) = limit {
        matches.truncate(n);
    }

    Ok(json!({
        "matches": matches,
        "count": matches.len()
    }))
}

fn project_value(v: Value, project: &[Value]) -> Value {
    let mut out = serde_json::Map::new();
    for key in project.iter().filter_map(Value::as_str) {
        if let Some(val) = v.get(key) {
            out.insert(key.to_string(), val.clone());
        }
    }
    Value::Object(out)
}

fn value_for_sort(v: &Value, key: &str) -> f64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_v2_supports_projection_sort_and_limit() {
        let p = std::env::temp_dir().join(format!("etr-jqv2-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(
            &p,
            r#"{"items":[{"id":1,"score":10},{"id":2,"score":30},{"id":3,"score":20}]}"#,
        )
        .unwrap();
        let out = run(&json!({
            "path": p,
            "query": ".items[*]",
            "project": ["id", "score"],
            "sort_by": "score",
            "sort_order": "desc",
            "limit": 2
        }))
        .unwrap();
        assert_eq!(out["count"], 2);
        assert_eq!(out["matches"][0]["id"], 2);
        let _ = std::fs::remove_file(&p);
    }
}

