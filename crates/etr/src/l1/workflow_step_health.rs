use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let text = std::fs::read_to_string(path)?;
    let root: Value = serde_json::from_str(&text)?;

    let steps = root
        .pointer("/state/steps")
        .or_else(|| root.get("steps"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut failed = Vec::new();
    let mut running = 0usize;
    for (id, s) in steps {
        let status = s
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        if status.eq_ignore_ascii_case("running") {
            running += 1;
        }
        if status.eq_ignore_ascii_case("failed") {
            failed.push(json!({
                "id": id,
                "retries": s.get("retries").cloned().unwrap_or(Value::from(0)),
                "error": s.get("error").cloned().unwrap_or(Value::Null)
            }));
        }
    }
    Ok(json!({
        "running_steps": running,
        "failed_steps": failed,
        "failed_count": failed.len()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_failed_steps() {
        let p = std::env::temp_dir().join(format!("etr-wsh-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(
            &p,
            r#"{"steps":{"a":{"status":"failed","retries":2},"b":{"status":"running"}}}"#,
        )
        .unwrap();
        let out = run(&json!({"path":p})).unwrap();
        assert_eq!(out["failed_count"], 1);
    }
}

