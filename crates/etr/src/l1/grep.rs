use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::io::BufRead;

pub fn run(input: &Value) -> Result<Value> {
    let pattern = input["pattern"].as_str().context("pattern required")?;
    let path = input["path"].as_str().context("path required")?;
    let context_lines = input
        .get("context_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let re = Regex::new(pattern).context("invalid regex pattern")?;
    let metadata =
        std::fs::metadata(path).context(format!("cannot access path: {path}"))?;

    let mut matches = Vec::new();

    if metadata.is_file() {
        search_file(&re, path, context_lines, &mut matches)?;
    } else if metadata.is_dir() {
        for entry in walkdir_simple(path) {
            if let Err(e) = search_file(&re, &entry, context_lines, &mut matches) {
                tracing::debug!("skipping {entry}: {e}");
            }
        }
    }

    let count = matches.len();
    Ok(json!({ "matches": matches, "count": count }))
}

fn search_file(
    re: &Regex,
    path: &str,
    context_lines: usize,
    out: &mut Vec<Value>,
) -> Result<()> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let lines: Vec<String> = reader.lines().collect::<std::result::Result<_, _>>()?;

    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            let start = i.saturating_sub(context_lines);
            let end = (i + context_lines + 1).min(lines.len());
            let ctx_before: Vec<&str> =
                lines[start..i].iter().map(|s| s.as_str()).collect();
            let ctx_after: Vec<&str> =
                lines[(i + 1)..end].iter().map(|s| s.as_str()).collect();

            out.push(json!({
                "file": path,
                "line": i + 1,
                "text": line,
                "context_before": ctx_before,
                "context_after": ctx_after,
            }));
        }
    }
    Ok(())
}

fn walkdir_simple(root: &str) -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Some(s) = p.to_str() {
                    paths.push(s.to_string());
                }
            } else if p.is_dir() {
                if p.file_name()
                    .map(|n| !n.to_string_lossy().starts_with('.'))
                    .unwrap_or(false)
                {
                    paths.extend(walkdir_simple(p.to_str().unwrap_or("")));
                }
            }
        }
    }
    paths
}
