use crate::budget;
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};
pub fn run(input: &Value) -> Result<Value> {
    let pattern = input["pattern"].as_str().context("pattern required")?;
    let path = input["path"].as_str().context("path required")?;
    let context = input["context_lines"].as_u64().unwrap_or(0).min(20) as usize;
    let re = Regex::new(pattern).context("invalid regex pattern")?;
    let root = std::path::Path::new(path);
    if std::fs::symlink_metadata(root)?.file_type().is_symlink() {
        anyhow::bail!("Symlink input is not allowed");
    }
    let (files, mut truncated) = if root.is_file() {
        (vec![root.to_path_buf()], false)
    } else {
        budget::files(root)
    };
    let start = std::time::Instant::now();
    let mut matches = Vec::new();
    let mut size = 0;
    let mut bytes = 0;
    'files: for file in files {
        if start.elapsed() > std::time::Duration::from_secs(2) {
            truncated = true;
            break;
        }
        let Ok(text) = budget::read_to_string(&file) else {
            truncated = true;
            continue;
        };
        bytes += text.len() as u64;
        if bytes > budget::TOTAL_BYTES {
            truncated = true;
            break;
        }
        let lines: Vec<_> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if start.elapsed() > std::time::Duration::from_secs(2) {
                truncated = true;
                break 'files;
            }
            if !re.is_match(line) {
                continue;
            }
            let value = json!({"file":file,"line":i+1,"text":line,"context_before":lines[i.saturating_sub(context)..i],"context_after":lines[i+1..(i+context+1).min(lines.len())]});
            size += value.to_string().len();
            if matches.len() >= budget::RESULTS || size > budget::OUTPUT_BYTES {
                truncated = true;
                break 'files;
            }
            matches.push(value);
        }
    }
    Ok(json!({"count":matches.len(),"matches":matches,"truncated":truncated}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn results_are_bounded_before_compaction() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("many");
        std::fs::write(&p, "needle\n".repeat(2000)).unwrap();
        let r = run(&json!({"path":p,"pattern":"needle"})).unwrap();
        assert_eq!(r["count"], 512);
        assert_eq!(r["truncated"], true);
    }
}
