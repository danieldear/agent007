use anyhow::{Context, Result};
use similar::{ChangeTag, TextDiff};
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path_a = input["path_a"].as_str().context("path_a required")?;
    let path_b = input["path_b"].as_str().context("path_b required")?;

    let text_a =
        std::fs::read_to_string(path_a).context(format!("cannot read {path_a}"))?;
    let text_b =
        std::fs::read_to_string(path_b).context(format!("cannot read {path_b}"))?;

    let diff = TextDiff::from_lines(&text_a, &text_b);

    let mut unified = String::new();
    let mut changed_lines = 0usize;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {
                unified.push_str(&format!("- {}", change));
                changed_lines += 1;
            }
            ChangeTag::Insert => {
                unified.push_str(&format!("+ {}", change));
                changed_lines += 1;
            }
            ChangeTag::Equal => {
                unified.push_str(&format!("  {}", change));
            }
        }
    }

    Ok(json!({
        "unified_diff": unified,
        "changed_lines": changed_lines,
    }))
}
