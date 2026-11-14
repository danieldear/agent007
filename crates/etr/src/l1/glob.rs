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

    let (files, mut truncated) = crate::budget::files(Path::new(root));
    let mut paths = Vec::new();
    for p in files {
        if set.is_match(p.strip_prefix(root).unwrap_or(&p)) {
            if paths.len() >= crate::budget::RESULTS {
                truncated = true;
                break;
            }
            paths.push(p.to_string_lossy().into_owned());
        }
    }
    Ok(json!({"count":paths.len(),"paths":paths,"truncated":truncated}))
}
