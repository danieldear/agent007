use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let mode = input["mode"].as_str().unwrap_or("text");
    let max_bytes = input["max_bytes"].as_u64().unwrap_or(100_000) as usize;

    let bytes = std::fs::read(path).context(format!("cannot read {path}"))?;
    let truncated = bytes.len() > max_bytes;
    let slice = if truncated {
        &bytes[..max_bytes]
    } else {
        &bytes[..]
    };

    match mode {
        "text" => {
            let text = String::from_utf8_lossy(slice).to_string();
            Ok(json!({
                "path": path,
                "mode": "text",
                "size_bytes": bytes.len(),
                "truncated": truncated,
                "text": text
            }))
        }
        "json" => {
            if truncated {
                anyhow::bail!(
                    "artifact exceeds max_bytes for JSON mode; increase max_bytes to parse full JSON"
                );
            }
            let v: Value = serde_json::from_slice(slice).context("invalid JSON artifact")?;
            Ok(json!({
                "path": path,
                "mode": "json",
                "size_bytes": bytes.len(),
                "truncated": truncated,
                "value": v
            }))
        }
        other => anyhow::bail!("unsupported mode '{other}' (use text or json)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_artifact() {
        let p = std::env::temp_dir().join(format!("etr-art-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&p, "hello").unwrap();
        let out = run(&json!({"path": p, "mode":"text"})).unwrap();
        assert_eq!(out["text"], "hello");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn reads_json_artifact() {
        let p = std::env::temp_dir().join(format!("etr-art-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&p, r#"{"ok":true}"#).unwrap();
        let out = run(&json!({"path": p, "mode":"json"})).unwrap();
        assert_eq!(out["value"]["ok"], true);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn json_mode_rejects_truncated_parse() {
        let p = std::env::temp_dir().join(format!("etr-art-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&p, r#"{"big":"0123456789"}"#).unwrap();
        let err = run(&json!({"path": p, "mode":"json", "max_bytes": 5})).unwrap_err();
        assert!(
            err.to_string().contains("exceeds max_bytes"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_file(&p);
    }
}
