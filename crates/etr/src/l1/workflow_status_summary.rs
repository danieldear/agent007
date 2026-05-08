use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let content = std::fs::read_to_string(path).context(format!("cannot read {path}"))?;
    let root: Value = serde_json::from_str(&content).context("invalid JSON")?;

    let progress = root.get("progress").unwrap_or(&root);
    let completed = progress
        .get("completed_steps")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = progress
        .get("total_steps")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_error = progress.get("last_error").cloned().unwrap_or(Value::Null);
    let pending = progress
        .get("pending_approval")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let running_steps = collect_step_ids(progress.get("running_steps"));
    let ready_steps = collect_step_ids(progress.get("ready_steps"));
    let outputs_available = progress
        .get("outputs_available")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(ToString::to_string).collect::<Vec<_>>())
        .unwrap_or_default();

    Ok(json!({
        "completed_steps": completed,
        "total_steps": total,
        "running_steps": running_steps,
        "ready_steps": ready_steps,
        "pending_approval": pending,
        "last_error": last_error,
        "outputs_available": outputs_available,
    }))
}

fn collect_step_ids(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|x| {
                    x.get("id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .or_else(|| x.as_str().map(ToString::to_string))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_progress_fields() {
        let p = std::env::temp_dir().join(format!("etr-wf-status-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(
            &p,
            r#"{"progress":{"completed_steps":2,"total_steps":5,"ready_steps":[{"id":"x"}],"running_steps":[{"id":"y"}],"outputs_available":["a"],"pending_approval":"gate","last_error":null}}"#,
        )
        .unwrap();
        let out = run(&json!({"path": p})).unwrap();
        assert_eq!(out["completed_steps"], 2);
        assert_eq!(out["ready_steps"][0], "x");
        let _ = std::fs::remove_file(&p);
    }
}

