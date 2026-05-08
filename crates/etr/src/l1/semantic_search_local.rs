use anyhow::{Context, Result};
use globset::{Glob, GlobSetBuilder};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub fn run(input: &Value) -> Result<Value> {
    let root = input["root"].as_str().unwrap_or(".");
    let query = input["query"].as_str().context("query required")?;
    let pattern = input["pattern"].as_str().unwrap_or("**/*.{md,rs,toml,txt,json,yaml,yml}");
    let limit = input["limit"].as_u64().unwrap_or(10) as usize;

    let mut b = GlobSetBuilder::new();
    b.add(Glob::new(pattern)?);
    let set = b.build()?;

    let q_tokens = tokens(query);
    let mut results = Vec::new();
    walk(Path::new(root), &set, &q_tokens, limit, &mut results)?;
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
        "results": results
    }))
}

fn walk(
    dir: &Path,
    set: &globset::GlobSet,
    q_tokens: &HashSet<String>,
    limit: usize,
    out: &mut Vec<Value>,
) -> Result<()> {
    if out.len() >= limit * 5 {
        return Ok(());
    }
    for ent in std::fs::read_dir(dir).context(format!("cannot read {}", dir.display()))? {
        let ent = ent?;
        let p = ent.path();
        if p.file_name().and_then(|s| s.to_str()) == Some(".git")
            || p.file_name().and_then(|s| s.to_str()) == Some("target")
        {
            continue;
        }
        if p.is_dir() {
            walk(&p, set, q_tokens, limit, out)?;
            continue;
        }
        if !set.is_match(&p) {
            continue;
        }
        let content = match std::fs::read_to_string(&p) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let c_tokens = tokens(&content);
        let overlap = q_tokens.intersection(&c_tokens).count();
        if overlap == 0 {
            continue;
        }
        let denom = q_tokens.len().max(1) as f64;
        let score = overlap as f64 / denom;
        out.push(json!({
            "path": p.to_string_lossy().to_string(),
            "score": score,
            "overlap_tokens": overlap
        }));
    }
    Ok(())
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

