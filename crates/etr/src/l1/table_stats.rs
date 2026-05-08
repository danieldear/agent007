use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let format = input["format"].as_str().unwrap_or("auto");
    let max_distinct = input["max_distinct"].as_u64().unwrap_or(20) as usize;

    let format = detect_format(path, format);
    match format.as_str() {
        "csv" => stats_csv(path, max_distinct),
        "jsonl" => stats_jsonl(path, max_distinct),
        other => anyhow::bail!("unsupported format '{other}' (use csv or jsonl)"),
    }
}

fn detect_format(path: &str, explicit: &str) -> String {
    if explicit != "auto" {
        return explicit.to_string();
    }
    if path.ends_with(".jsonl") || path.ends_with(".ndjson") {
        "jsonl".to_string()
    } else {
        "csv".to_string()
    }
}

fn stats_csv(path: &str, max_distinct: usize) -> Result<Value> {
    let mut rdr = csv::Reader::from_path(path).context(format!("cannot read {path}"))?;
    let headers = rdr.headers()?.clone();
    let mut rows = 0usize;
    let mut null_counts = vec![0usize; headers.len()];
    let mut distinct: Vec<HashMap<String, usize>> = vec![HashMap::new(); headers.len()];

    for rec in rdr.records() {
        let rec = rec?;
        rows += 1;
        for (i, field) in rec.iter().enumerate() {
            if field.trim().is_empty() {
                null_counts[i] += 1;
            }
            if distinct[i].len() < max_distinct || distinct[i].contains_key(field) {
                *distinct[i].entry(field.to_string()).or_insert(0) += 1;
            }
        }
    }

    let mut columns = Vec::new();
    for (i, name) in headers.iter().enumerate() {
        let mut top = BTreeMap::new();
        for (k, v) in &distinct[i] {
            top.insert(k.clone(), *v);
        }
        columns.push(json!({
            "name": name,
            "null_count": null_counts[i],
            "distinct_sample_count": top.len(),
            "distinct_sample": top
        }));
    }

    Ok(json!({
        "format": "csv",
        "rows": rows,
        "column_count": headers.len(),
        "columns": columns
    }))
}

fn stats_jsonl(path: &str, max_distinct: usize) -> Result<Value> {
    let content = std::fs::read_to_string(path).context(format!("cannot read {path}"))?;
    let mut rows = 0usize;
    let mut columns: HashMap<String, (usize, HashMap<String, usize>)> = HashMap::new();

    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        rows += 1;
        let v: Value = serde_json::from_str(line).context("invalid JSONL line")?;
        let obj = v.as_object().context("JSONL line must be object")?;
        for (k, vv) in obj {
            let entry = columns
                .entry(k.clone())
                .or_insert_with(|| (0usize, HashMap::new()));
            if vv.is_null() {
                entry.0 += 1;
            }
            let s = scalar_to_string(vv);
            if entry.1.len() < max_distinct || entry.1.contains_key(&s) {
                *entry.1.entry(s).or_insert(0) += 1;
            }
        }
    }

    let mut cols_out = Vec::new();
    let mut names: Vec<_> = columns.keys().cloned().collect();
    names.sort();
    for name in names {
        let (null_count, values) = columns.remove(&name).unwrap_or_default();
        let mut sample = BTreeMap::new();
        for (k, v) in values {
            sample.insert(k, v);
        }
        cols_out.push(json!({
            "name": name,
            "null_count": null_count,
            "distinct_sample_count": sample.len(),
            "distinct_sample": sample
        }));
    }

    Ok(json!({
        "format": "jsonl",
        "rows": rows,
        "column_count": cols_out.len(),
        "columns": cols_out
    }))
}

fn scalar_to_string(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_stats_basic() {
        let p = std::env::temp_dir().join(format!("etr-table-stats-{}.csv", uuid::Uuid::new_v4()));
        std::fs::write(&p, "name,kind\nalice,error\nbob,\n").unwrap();
        let out = run(&json!({"path": p, "format":"csv"})).unwrap();
        assert_eq!(out["rows"], 2);
        assert_eq!(out["column_count"], 2);
        let _ = std::fs::remove_file(&p);
    }
}
