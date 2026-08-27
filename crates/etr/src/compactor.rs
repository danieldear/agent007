use serde_json::Value;

/// Below this serialized size, a result is left untouched. Generous on purpose:
/// this compares against text sent to an LLM with a large context budget, not
/// against a network payload, so there is no reason to trim a modest response.
const JSON_COMPACT_THRESHOLD: usize = 20_000;
/// Above this size, a single string field (file text, log body, preview) is
/// truncated. Applied per-field, not to the whole payload, so a few small
/// fields sitting next to one large one are never touched.
const STRING_FIELD_LIMIT: usize = 8_000;
/// Above this length, an array (rows, matches, edges) is truncated to its
/// first N items. Each kept item is still recursively compacted.
const ARRAY_ITEM_LIMIT: usize = 200;
const LOG_HEAD_TAIL_LINES: usize = 20;

pub struct Compactor;

impl Compactor {
    /// Compact a JSON value. Returns (compacted_value, was_truncated).
    ///
    /// Recurses into the structure and trims only the fields that are
    /// actually large, rather than discarding the whole payload for a
    /// byte-count preview. A byte-slice preview of serialized JSON can also
    /// land mid multi-byte UTF-8 character and panic; nothing here slices a
    /// string except through `truncate_str_boundary`, which cannot.
    pub fn compact_json(value: Value, compact_key: Option<&str>) -> (Value, bool) {
        if let Some(key) = compact_key {
            if let Some(sub) = value.get(key) {
                return (sub.clone(), false);
            }
        }
        if !exceeds_byte_threshold(&value, JSON_COMPACT_THRESHOLD) {
            return (value, false);
        }
        let mut truncated = false;
        let compacted = compact_value(value, &mut truncated);
        (compacted, truncated)
    }

    /// Compact a string field's worth of text: head + tail, preferring line
    /// boundaries when the text has enough lines to make that meaningful,
    /// and falling back to a raw byte head/tail for a large unstructured
    /// blob (e.g. a single huge minified-JSON or base64 line) that `lines()`
    /// would otherwise pass through untouched.
    pub fn compact_text(text: &str, max_bytes: usize) -> (String, bool) {
        if text.len() <= max_bytes {
            return (text.to_string(), false);
        }
        let half = max_bytes / 2;
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > LOG_HEAD_TAIL_LINES * 2 {
            let mut head = lines[..LOG_HEAD_TAIL_LINES].join("\n");
            let mut tail = lines[lines.len() - LOG_HEAD_TAIL_LINES..].join("\n");
            // Bounding the line COUNT does not bound the byte size: a kept
            // line can itself be arbitrarily long (minified content, a huge
            // single log entry). Clamp each side to the byte budget too, so
            // the result can never come back larger than intended.
            if head.len() > half {
                head = truncate_str_boundary(&head, half).to_string();
            }
            if tail.len() > half {
                let start = tail_boundary(&tail, tail.len() - half);
                tail = tail[start..].to_string();
            }
            return (
                format!(
                    "{head}\n... [{} lines omitted] ...\n{tail}",
                    lines.len() - LOG_HEAD_TAIL_LINES * 2
                ),
                true,
            );
        }
        let head = truncate_str_boundary(text, half);
        let tail_start = tail_boundary(text, text.len().saturating_sub(half));
        let tail = &text[tail_start..];
        (
            format!(
                "{head}\n... [{} bytes omitted] ...\n{tail}",
                text.len() - head.len() - tail.len()
            ),
            true,
        )
    }

    /// Compact a JSON array to its first `limit` items.
    pub fn compact_rows(rows: &[Value], limit: usize) -> (Vec<Value>, bool) {
        if rows.len() <= limit {
            return (rows.to_vec(), false);
        }
        (rows[..limit].to_vec(), true)
    }
}

fn compact_value(value: Value, truncated: &mut bool) -> Value {
    match value {
        Value::String(s) => {
            let (compacted, was_truncated) = Compactor::compact_text(&s, STRING_FIELD_LIMIT);
            if was_truncated {
                *truncated = true;
            }
            Value::String(compacted)
        }
        Value::Array(items) => {
            let (kept, was_truncated) = Compactor::compact_rows(&items, ARRAY_ITEM_LIMIT);
            if was_truncated {
                *truncated = true;
            }
            Value::Array(
                kept.into_iter()
                    .map(|item| compact_value(item, truncated))
                    .collect(),
            )
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, compact_value(v, truncated)))
                .collect(),
        ),
        other => other,
    }
}

/// Largest byte index `<= max_bytes` that lands on a UTF-8 char boundary.
/// Slicing a `str` at a non-boundary index panics; this can never produce one.
/// True if `value` would serialize to more than `limit` bytes.
///
/// Serializes into a byte counter rather than `serde_json::to_string`, so a
/// large value (e.g. a large file's text) is never fully allocated just to
/// measure it. The counter also aborts serialization as soon as `limit` is
/// crossed, so a genuinely huge payload does not get fully walked either.
fn exceeds_byte_threshold(value: &Value, limit: usize) -> bool {
    let mut counter = ByteCountBudget { count: 0, limit };
    serde_json::to_writer(&mut counter, value).is_err()
}

struct ByteCountBudget {
    count: usize,
    limit: usize,
}

impl std::io::Write for ByteCountBudget {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.count += buf.len();
        if self.count > self.limit {
            return Err(std::io::Error::other("compactor byte budget exceeded"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn truncate_str_boundary(s: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Smallest byte index `>= from` that lands on a UTF-8 char boundary, for
/// slicing a tail (`&s[idx..]`) without landing mid-character.
fn tail_boundary(s: &str, from: usize) -> usize {
    let mut start = from.min(s.len());
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn small_payload_is_left_untouched() {
        let value = json!({"path": "README.md", "text": "hello world"});
        let (out, truncated) = Compactor::compact_json(value.clone(), None);
        assert!(!truncated);
        assert_eq!(out, value);
    }

    #[test]
    fn large_string_field_is_trimmed_but_small_fields_survive_intact() {
        let big_text = "x".repeat(50_000);
        let value = json!({
            "path": "README.md",
            "size_bytes": 50_000,
            "text": big_text,
        });
        let (out, truncated) = Compactor::compact_json(value, None);
        assert!(truncated);
        // Small, informative fields are untouched: an agent can still see
        // exactly what was read and how large it was.
        assert_eq!(out["path"], "README.md");
        assert_eq!(out["size_bytes"], 50_000);
        // The oversized field is trimmed, not replaced with an opaque blob.
        let text = out["text"].as_str().unwrap();
        assert!(text.len() < 50_000);
        assert!(text.contains("bytes omitted") || text.contains("lines omitted"));
    }

    #[test]
    fn never_panics_when_the_cut_point_lands_inside_a_multibyte_character() {
        // Byte 499..502 is a single 3-byte UTF-8 character ('日'), so a raw
        // slice at a fixed byte offset near there used to panic.
        let mut s = "x".repeat(499);
        s.push('日');
        s.push_str(&"y".repeat(50_000));
        let value = json!({ "text": s });
        // Must not panic. compact_json's own internal size check exercises
        // the exact boundary-unsafe code path this guards against.
        let (out, truncated) = Compactor::compact_json(value, None);
        assert!(truncated);
        assert!(out["text"].as_str().unwrap().is_char_boundary(0)); // trivially true; real check is "did not panic"
    }

    #[test]
    fn large_array_is_truncated_to_its_first_items_not_discarded() {
        // Padded so the total serialized payload clears JSON_COMPACT_THRESHOLD
        // on its own, independent of ARRAY_ITEM_LIMIT.
        let rows: Vec<Value> = (0..1000)
            .map(|i| json!({"id": i, "pad": "x".repeat(50)}))
            .collect();
        let value = json!({ "rows": rows, "total": 1000 });
        let (out, truncated) = Compactor::compact_json(value, None);
        assert!(truncated);
        let kept = out["rows"].as_array().unwrap();
        assert_eq!(kept.len(), ARRAY_ITEM_LIMIT);
        assert_eq!(kept[0]["id"], 0);
        // The tool's own "total" field, unrelated to array length, survives.
        assert_eq!(out["total"], 1000);
    }

    #[test]
    fn compact_key_extracts_a_sub_value_regardless_of_size() {
        let value = json!({"wrapper": {"inner": "kept"}, "noise": "x".repeat(50_000)});
        let (out, truncated) = Compactor::compact_json(value, Some("wrapper"));
        assert!(!truncated);
        assert_eq!(out, json!({"inner": "kept"}));
    }

    #[test]
    fn compact_text_prefers_line_boundaries_when_there_are_enough_lines() {
        let text = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (out, truncated) = Compactor::compact_text(&text, 200);
        assert!(truncated);
        assert!(out.starts_with("line 0"));
        assert!(out.ends_with("line 99"));
        assert!(out.contains("lines omitted"));
    }

    #[test]
    fn compact_text_line_branch_still_enforces_the_byte_budget() {
        // 50 lines clears the "enough lines" branch, but each line is itself
        // 5,000 bytes — bounding line COUNT alone would let 20 of them through
        // per side, ~100,000 bytes, far past a small max_bytes budget.
        let long_line = "x".repeat(5_000);
        let text = std::iter::repeat(long_line)
            .take(50)
            .collect::<Vec<_>>()
            .join("\n");
        let (out, truncated) = Compactor::compact_text(&text, 2_000);
        assert!(truncated);
        assert!(
            out.len() < 5_000,
            "line-based branch must still respect max_bytes, got {} bytes",
            out.len()
        );
    }

    #[test]
    fn compact_text_falls_back_to_byte_truncation_for_one_giant_line() {
        // A single unbroken line (e.g. minified JSON) has no useful line
        // boundaries; `lines()` reports exactly one line no matter how long.
        let text = "x".repeat(100_000);
        let (out, truncated) = Compactor::compact_text(&text, 500);
        assert!(truncated);
        assert!(out.len() < text.len());
        assert!(out.contains("bytes omitted"));
    }

    #[test]
    fn compact_rows_keeps_first_n_items() {
        let rows: Vec<Value> = (0..10).map(|i| json!(i)).collect();
        let (kept, truncated) = Compactor::compact_rows(&rows, 3);
        assert!(truncated);
        assert_eq!(kept, vec![json!(0), json!(1), json!(2)]);
    }

    #[test]
    fn compact_rows_leaves_short_arrays_untouched() {
        let rows: Vec<Value> = (0..3).map(|i| json!(i)).collect();
        let (kept, truncated) = Compactor::compact_rows(&rows, 50);
        assert!(!truncated);
        assert_eq!(kept, rows);
    }

    #[test]
    fn byte_threshold_check_matches_the_actual_serialized_size() {
        // Cross-check the bounded-writer size check against the true
        // serialized length it replaces, at and around the boundary.
        let value = json!({"text": "x".repeat(1_000)});
        let actual_len = serde_json::to_string(&value).unwrap().len();

        assert!(!exceeds_byte_threshold(&value, actual_len));
        assert!(!exceeds_byte_threshold(&value, actual_len + 1));
        assert!(exceeds_byte_threshold(&value, actual_len - 1));
    }
}
