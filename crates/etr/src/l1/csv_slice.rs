use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
    let columns: Option<Vec<&str>> = input
        .get("columns")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

    let mut rdr = csv::Reader::from_path(path).context(format!("cannot read CSV: {path}"))?;
    let headers: Vec<String> = rdr.headers()?.iter().map(|s| s.to_string()).collect();

    let col_indices: Option<Vec<usize>> = columns.as_ref().map(|cols| {
        cols.iter()
            .filter_map(|c| headers.iter().position(|h| h == c))
            .collect()
    });

    let mut rows: Vec<Value> = Vec::new();
    let mut total = 0usize;

    for result in rdr.records() {
        let record = result?;
        total += 1;
        if rows.len() < limit {
            let row: serde_json::Map<String, Value> = if let Some(ref idxs) = col_indices {
                idxs.iter()
                    .filter_map(|&i| {
                        headers
                            .get(i)
                            .zip(record.get(i))
                            .map(|(h, v)| (h.clone(), json!(v)))
                    })
                    .collect()
            } else {
                headers
                    .iter()
                    .zip(record.iter())
                    .map(|(h, v)| (h.clone(), json!(v)))
                    .collect()
            };
            rows.push(Value::Object(row));
        }
    }

    let effective_headers: Vec<&str> = if let Some(ref cols) = columns {
        cols.clone()
    } else {
        headers.iter().map(|s| s.as_str()).collect()
    };

    Ok(json!({
        "rows": rows,
        "total_rows": total,
        "columns": effective_headers,
        "truncated": total > limit,
    }))
}
