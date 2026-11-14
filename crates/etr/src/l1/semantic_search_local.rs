use anyhow::{Context, Result};
use globset::{Glob, GlobSetBuilder};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub fn run(input: &Value) -> Result<Value> {
    let root = input["root"].as_str().unwrap_or(".");
    let query = input["query"].as_str().context("query required")?;
    let pattern = input["pattern"]
        .as_str()
        .unwrap_or("**/*.{md,rs,toml,txt,json,yaml,yml}");
    let limit = input["limit"]
        .as_u64()
        .unwrap_or(10)
        .min(crate::budget::RESULTS as u64) as usize;

    let mut b = GlobSetBuilder::new();
    b.add(Glob::new(pattern)?);
    let set = b.build()?;

    let q_tokens = tokens(query);
    let mut results = Vec::new();
    let (files, mut truncated) = crate::budget::files(Path::new(root));
    let started = std::time::Instant::now();
    let mut bytes = 0;
    for p in files {
        if started.elapsed() > std::time::Duration::from_secs(2) {
            truncated = true;
            break;
        }
        if !set.is_match(p.strip_prefix(root).unwrap_or(&p)) {
            continue;
        }
        let Ok(content) = crate::budget::read_to_string(&p) else {
            continue;
        };
        bytes += content.len();
        if bytes > crate::budget::TOTAL_BYTES as usize {
            truncated = true;
            break;
        }
        let overlap = q_tokens.intersection(&tokens(&content)).count();
        if overlap == 0 {
            continue;
        }
        results.push(json!({"path":p.to_string_lossy(),"score":overlap as f64/q_tokens.len().max(1) as f64,"overlap_tokens":overlap}));
        if results.len() >= crate::budget::RESULTS {
            truncated = true;
            break;
        }
    }
    results.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .partial_cmp(&a["score"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if results.len() > limit {
        results.truncate(limit);
    }

    Ok(json!({
        "query": query,
        "root": root,
        "count": results.len(),
        "truncated": truncated,
        "results": results
    }))
}

fn tokens(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 3)
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_ranked_matches() {
        let dir = std::env::temp_dir().join(format!("etr-sem-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "workflow status summary and errors").unwrap();
        std::fs::write(dir.join("b.txt"), "nothing relevant").unwrap();
        let out = run(&json!({
            "root": dir,
            "query":"workflow summary errors",
            "pattern":"**/*.txt"
        }))
        .unwrap();
        assert!(out["count"].as_u64().unwrap_or(0) >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
