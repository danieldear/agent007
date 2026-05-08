use serde_json::Value;

const JSON_COMPACT_THRESHOLD: usize = 2048;
const LOG_HEAD_TAIL_LINES: usize = 20;
const TABULAR_ROW_LIMIT: usize = 50;

pub struct Compactor;

impl Compactor {
    /// Compact a JSON value. Returns (compacted_value, was_truncated).
    pub fn compact_json(value: Value, compact_key: Option<&str>) -> (Value, bool) {
        if let Some(key) = compact_key {
            if let Some(sub) = value.get(key) {
                return (sub.clone(), false);
            }
        }
        let serialized = serde_json::to_string(&value).unwrap_or_default();
        if serialized.len() <= JSON_COMPACT_THRESHOLD {
            (value, false)
        } else {
            (
                serde_json::json!({
                    "truncated": true,
                    "size_bytes": serialized.len(),
                    "preview": &serialized[..serialized.len().min(500)]
                }),
                true,
            )
        }
    }

    /// Compact log/text output (head + tail).
    pub fn compact_text(text: &str) -> (String, bool) {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= LOG_HEAD_TAIL_LINES * 2 {
            return (text.to_string(), false);
        }
        let head: Vec<&str> = lines[..LOG_HEAD_TAIL_LINES].to_vec();
        let tail: Vec<&str> = lines[lines.len() - LOG_HEAD_TAIL_LINES..].to_vec();
        let compacted = format!(
            "{}\n... [{} lines omitted] ...\n{}",
            head.join("\n"),
            lines.len() - LOG_HEAD_TAIL_LINES * 2,
            tail.join("\n")
        );
        (compacted, true)
    }

    /// Compact tabular rows (first N + summary).
    pub fn compact_rows(rows: &[serde_json::Value], total: usize) -> (Vec<serde_json::Value>, bool) {
        if rows.len() <= TABULAR_ROW_LIMIT {
            return (rows.to_vec(), false);
        }
        let _ = total; // total is informational for callers
        (rows[..TABULAR_ROW_LIMIT].to_vec(), true)
    }
}
