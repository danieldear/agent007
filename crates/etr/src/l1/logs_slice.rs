use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let level = input["level"].as_str().map(|s| s.to_ascii_lowercase());
    let contains = input["contains"].as_str();
    let max_lines = input["max_lines"].as_u64().unwrap_or(200) as usize;

    let content = std::fs::read_to_string(path).context(format!("cannot read {path}"))?;
    let level_re = Regex::new(r"\b(trace|debug|info|warn|error|fatal)\b").unwrap();

    let mut lines = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if let Some(ref lv) = level {
            let found = level_re
                .find(&line.to_ascii_lowercase())
                .map(|m| m.as_str().to_string());
            if found.as_deref() != Some(lv.as_str()) {
                continue;
            }
        }
        if let Some(sub) = contains {
            if !line.contains(sub) {
                continue;
            }
        }
        lines.push(json!({
            "line": i + 1,
            "text": line
        }));
        if lines.len() >= max_lines {
            break;
        }
    }

    Ok(json!({
        "path": path,
        "count": lines.len(),
        "max_lines": max_lines,
        "lines": lines
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_level_and_contains() {
        let p = std::env::temp_dir().join(format!("etr-logs-{}.log", uuid::Uuid::new_v4()));
        std::fs::write(&p, "INFO start\nERROR failed x\nWARN skip\n").unwrap();
        let out = run(&json!({"path":p, "level":"error", "contains":"failed"})).unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["lines"][0]["line"], 2);
        let _ = std::fs::remove_file(&p);
    }
}
