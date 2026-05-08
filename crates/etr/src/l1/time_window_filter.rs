use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let field = input["timestamp_field"]
        .as_str()
        .unwrap_or("timestamp")
        .to_string();
    let start = input["start"].as_str().context("start required")?;
    let end = input["end"].as_str().context("end required")?;
    let rows = if let Some(path) = input["path"].as_str() {
        super::table_select::run(&json!({"path":path,"format":input["format"]}))?["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    } else {
        input["rows"].as_array().cloned().unwrap_or_default()
    };
    let s = parse_ts(start)?;
    let e = parse_ts(end)?;
    let out = rows
        .into_iter()
        .filter(|r| {
            r.get(&field)
                .and_then(Value::as_str)
                .and_then(|t| parse_ts(t).ok())
                .map(|t| t >= s && t <= e)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    Ok(json!({"rows": out, "count": out.len()}))
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_window() {
        let out = run(&json!({
            "timestamp_field":"ts",
            "start":"2026-01-01T00:00:00Z",
            "end":"2026-01-01T00:00:10Z",
            "rows":[{"ts":"2026-01-01T00:00:05Z"},{"ts":"2026-01-01T00:01:00Z"}]
        }))
        .unwrap();
        assert_eq!(out["count"], 1);
    }
}

