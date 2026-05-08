use anyhow::Result;
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("path required"))?;
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mtime = meta
                .modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
                .unwrap_or_else(|| "unknown".to_string());
            Ok(json!({
                "exists": true,
                "size_bytes": meta.len(),
                "mtime": mtime,
                "is_file": meta.is_file(),
                "is_dir": meta.is_dir(),
            }))
        }
        Err(_) => Ok(json!({ "exists": false })),
    }
}
