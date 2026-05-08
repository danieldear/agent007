use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let path = input["path"].as_str().context("path required")?;
    let query = input["query"].as_str().context("query required")?;

    let content = std::fs::read_to_string(path).context(format!("cannot read {path}"))?;
    let doc: Value = serde_json::from_str(&content).context("invalid JSON")?;

    let matches = run_query(&doc, query)?;
    Ok(json!({
        "matches": matches,
        "count": matches.len()
    }))
}

fn run_query(doc: &Value, query: &str) -> Result<Vec<Value>> {
    let query = query.trim().trim_start_matches('.');
    if query.is_empty() {
        return Ok(vec![doc.clone()]);
    }

    let mut current = vec![doc.clone()];
    for segment in split_segments(query) {
        current = apply_segment(&current, segment.as_str())?;
    }
    Ok(current)
}

fn split_segments(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for ch in query.chars() {
        match ch {
            '.' if depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '[' => {
                depth += 1;
                cur.push(ch);
            }
            ']' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn apply_segment(values: &[Value], segment: &str) -> Result<Vec<Value>> {
    let (base, selector) = parse_segment(segment)?;
    let mut out = Vec::new();

    for value in values {
        let base_value = if base.is_empty() {
            value.clone()
        } else if let Some(v) = value.get(&base) {
            v.clone()
        } else {
            continue;
        };

        match &selector {
            Selector::None => out.push(base_value),
            Selector::Index(idx) => {
                if let Some(v) = base_value.get(*idx) {
                    out.push(v.clone());
                }
            }
            Selector::Wildcard => {
                if let Some(arr) = base_value.as_array() {
                    out.extend(arr.iter().cloned());
                }
            }
            Selector::Filter { key, expected } => {
                if let Some(arr) = base_value.as_array() {
                    for item in arr {
                        let Some(actual) = item.get(key) else {
                            continue;
                        };
                        if value_eq_expected(actual, expected) {
                            out.push(item.clone());
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn value_eq_expected(actual: &Value, expected: &str) -> bool {
    match actual {
        Value::String(s) => s == expected,
        Value::Number(n) => n.to_string() == expected,
        Value::Bool(b) => b.to_string() == expected,
        _ => false,
    }
}

enum Selector {
    None,
    Index(usize),
    Wildcard,
    Filter { key: String, expected: String },
}

fn parse_segment(segment: &str) -> Result<(String, Selector)> {
    let Some(open) = segment.find('[') else {
        return Ok((segment.to_string(), Selector::None));
    };
    let base = segment[..open].to_string();
    let close = segment
        .rfind(']')
        .context("invalid query segment: missing closing ']'")?;
    if close <= open {
        anyhow::bail!("invalid query segment: malformed selector");
    }
    let inner = &segment[open + 1..close];
    let selector = if inner == "*" {
        Selector::Wildcard
    } else if let Ok(idx) = inner.parse::<usize>() {
        Selector::Index(idx)
    } else if let Some((k, v)) = inner.split_once('=') {
        let expected = v.trim().trim_matches('"').trim_matches('\'').to_string();
        Selector::Filter {
            key: k.trim().to_string(),
            expected,
        }
    } else {
        anyhow::bail!("unsupported selector '[{inner}]'");
    };
    Ok((base, selector))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_with_wildcard_and_projection() {
        let doc = json!({"results":[{"score":10},{"score":20}]});
        let out = run_query(&doc, ".results[*].score").unwrap();
        assert_eq!(out, vec![json!(10), json!(20)]);
    }

    #[test]
    fn query_with_filter() {
        let doc = json!({"items":[{"kind":"error","id":1},{"kind":"info","id":2}]});
        let out = run_query(&doc, ".items[kind=error].id").unwrap();
        assert_eq!(out, vec![json!(1)]);
    }

    #[test]
    fn query_with_index() {
        let doc = json!({"items":[{"id":1},{"id":2}]});
        let out = run_query(&doc, ".items[1].id").unwrap();
        assert_eq!(out, vec![json!(2)]);
    }
}
