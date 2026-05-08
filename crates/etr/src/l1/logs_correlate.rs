use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn run(input: &Value) -> Result<Value> {
    let path_a = input["path_a"].as_str().context("path_a required")?;
    let path_b = input["path_b"].as_str().context("path_b required")?;
    let pattern = input["pattern"].as_str().context("pattern required")?;
    let group = input["group"].as_u64().unwrap_or(1) as usize;

    let re = Regex::new(pattern).context("invalid regex pattern")?;
    let a = collect(path_a, &re, group)?;
    let b = collect(path_b, &re, group)?;

    let mut matches = Vec::new();
    for (k, a_lines) in a {
        if let Some(b_lines) = b.get(&k) {
            matches.push(json!({"token": k, "lines_a": a_lines, "lines_b": b_lines}));
        }
    }
    Ok(json!({"matches": matches, "count": matches.len()}))
}

fn collect(path: &str, re: &Regex, group: usize) -> Result<HashMap<String, Vec<usize>>> {
    let text = std::fs::read_to_string(path)?;
    let mut out: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, line) in text.lines().enumerate() {
        for cap in re.captures_iter(line) {
            if let Some(m) = cap.get(group) {
                out.entry(m.as_str().to_string()).or_default().push(i + 1);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correlates_tokens() {
        let a = std::env::temp_dir().join(format!("etr-lca-{}.log", uuid::Uuid::new_v4()));
        let b = std::env::temp_dir().join(format!("etr-lcb-{}.log", uuid::Uuid::new_v4()));
        std::fs::write(&a, "id=abc\n").unwrap();
        std::fs::write(&b, "seen abc\n").unwrap();
        let out = run(&json!({"path_a":a,"path_b":b,"pattern":"(abc)"})).unwrap();
        assert_eq!(out["count"], 1);
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }
}
