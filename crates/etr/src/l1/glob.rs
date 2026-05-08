use anyhow::{Context, Result};
use globset::{Glob, GlobSetBuilder};
use serde_json::{json, Value};
use std::path::Path;

pub fn run(input: &Value) -> Result<Value> {
    let pattern = input["pattern"].as_str().context("pattern required")?;
    let root = input.get("root").and_then(|v| v.as_str()).unwrap_or(".");

    let glob = Glob::new(pattern).context("invalid glob pattern")?;
    let mut builder = GlobSetBuilder::new();
    builder.add(glob);
    let set = builder.build().context("failed to build glob set")?;

    let mut paths: Vec<String> = Vec::new();
    collect_paths(Path::new(root), root, &set, &mut paths);

    Ok(json!({ "paths": paths, "count": paths.len() }))
}

fn collect_paths(
    dir: &Path,
    root: &str,
    set: &globset::GlobSet,
    out: &mut Vec<String>,
) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            let rel = p.strip_prefix(root).unwrap_or(&p);
            if set.is_match(rel) {
                if let Some(s) = p.to_str() {
                    out.push(s.to_string());
                }
            }
            if p.is_dir()
                && p.file_name()
                    .map(|n| !n.to_string_lossy().starts_with('.'))
                    .unwrap_or(false)
            {
                collect_paths(&p, root, set, out);
            }
        }
    }
}
