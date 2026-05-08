use anyhow::Result;
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let rows = if let Some(path) = input["path"].as_str() {
        super::table_select::run(&json!({"path":path,"format":input["format"]}))?["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    } else {
        input["rows"].as_array().cloned().unwrap_or_default()
    };
    let columns = input["columns"].as_array().cloned().unwrap_or_default();
    let cols = if columns.is_empty() {
        infer_columns(&rows)
    } else {
        columns
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect()
    };
    let mut out = serde_json::Map::new();
    for c in cols {
        let mut vals = Vec::new();
        let mut nulls = 0usize;
        for r in &rows {
            match r.get(&c) {
                Some(v) => {
                    if let Some(n) = to_f64(v) {
                        vals.push(n);
                    } else {
                        nulls += 1;
                    }
                }
                None => nulls += 1,
            }
        }
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let summary = if vals.is_empty() {
            json!({"count":0,"null_count":nulls})
        } else {
            let count = vals.len();
            let mean = vals.iter().sum::<f64>() / count as f64;
            let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count as f64;
            let stddev = var.sqrt();
            let p = |q: f64| -> f64 {
                let idx = ((count - 1) as f64 * q).round() as usize;
                vals[idx]
            };
            json!({
                "count": count,
                "null_count": nulls,
                "min": vals[0],
                "max": vals[count-1],
                "mean": mean,
                "p50": p(0.50),
                "p95": p(0.95),
                "stddev": stddev
            })
        };
        out.insert(c, summary);
    }
    Ok(json!({"metrics": out}))
}

fn infer_columns(rows: &[Value]) -> Vec<String> {
    rows.first()
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}
fn to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summarizes_metrics() {
        let out = run(&json!({"rows":[{"x":1},{"x":2},{"x":3}],"columns":["x"]})).unwrap();
        assert_eq!(out["metrics"]["x"]["p50"], 2.0);
    }
}
