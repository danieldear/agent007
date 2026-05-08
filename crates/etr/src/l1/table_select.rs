use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let format = input["format"].as_str().unwrap_or("auto");
    let limit = input["limit"].as_u64().map(|n| n as usize);
    let columns = input["columns"].as_array().cloned().unwrap_or_default();
    let where_obj = input["where"].as_object().cloned().unwrap_or_default();
    let order_by = input["order_by"].as_str();
    let order_desc = input["order_desc"].as_bool().unwrap_or(false);

    let mut rows = load_rows(path, format)?;
    rows.retain(|r| matches_where(r, &where_obj));
    if let Some(field) = order_by {
        rows.sort_by(|a, b| {
            let av = a.get(field).cloned().unwrap_or(Value::Null).to_string();
            let bv = b.get(field).cloned().unwrap_or(Value::Null).to_string();
            av.cmp(&bv)
        });
        if order_desc {
            rows.reverse();
        }
    }
    if !columns.is_empty() {
        rows = rows
            .into_iter()
            .map(|r| {
                let mut out = Map::new();
                for c in columns.iter().filter_map(Value::as_str) {
                    if let Some(v) = r.get(c) {
                        out.insert(c.to_string(), v.clone());
                    }
                }
                out
            })
            .collect();
    }
    if let Some(n) = limit {
        rows.truncate(n);
    }
    Ok(json!({"rows": rows, "count": rows.len()}))
}

fn matches_where(row: &Map<String, Value>, where_obj: &Map<String, Value>) -> bool {
    where_obj.iter().all(|(k, v)| row.get(k) == Some(v))
}

fn load_rows(path: &str, format: &str) -> Result<Vec<Map<String, Value>>> {
    let fmt = if format == "auto" {
        if path.ends_with(".jsonl") || path.ends_with(".ndjson") {
            "jsonl"
        } else {
            "csv"
        }
    } else {
        format
    };
    match fmt {
        "csv" => {
            let mut rdr = csv::Reader::from_path(path).context(format!("cannot read {path}"))?;
            let headers = rdr.headers()?.clone();
            let mut rows = Vec::new();
            for rec in rdr.records() {
                let rec = rec?;
                let mut m = Map::new();
                for (i, val) in rec.iter().enumerate() {
                    m.insert(headers[i].to_string(), Value::String(val.to_string()));
                }
                rows.push(m);
            }
            Ok(rows)
        }
        "jsonl" => {
            let text = std::fs::read_to_string(path)?;
            let mut rows = Vec::new();
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let v: Value = serde_json::from_str(line)?;
                if let Some(obj) = v.as_object() {
                    rows.push(obj.clone());
                }
            }
            Ok(rows)
        }
        _ => anyhow::bail!("unsupported format"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_rows() {
        let p = std::env::temp_dir().join(format!("etr-tsel-{}.csv", uuid::Uuid::new_v4()));
        std::fs::write(&p, "id,kind\n1,a\n2,b\n").unwrap();
        let out = run(&json!({"path":p,"where":{"kind":"b"},"columns":["id"]})).unwrap();
        assert_eq!(out["count"], 1);
    }
}
