use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value> {
    let pattern = input["pattern"].as_str().context("pattern required")?;
    let group = input["group"].as_u64().unwrap_or(0) as usize;
    let max_matches = input["max_matches"].as_u64().unwrap_or(100) as usize;
    let text = if let Some(s) = input["text"].as_str() {
        s.to_string()
    } else {
        let path = input["path"].as_str().context("path or text required")?;
        std::fs::read_to_string(path).context(format!("cannot read {path}"))?
    };

    let re = Regex::new(pattern).context("invalid regex pattern")?;
    let mut matches = Vec::new();

    for (line_idx, line) in text.lines().enumerate() {
        for cap in re.captures_iter(line) {
            let Some(m) = cap.get(group) else {
                continue;
            };
            matches.push(json!({
                "line": line_idx + 1,
                "start": m.start(),
                "end": m.end(),
                "value": m.as_str(),
            }));
            if matches.len() >= max_matches {
                return Ok(json!({
                    "matches": matches,
                    "count": matches.len(),
                    "truncated": true
                }));
            }
        }
    }

    Ok(json!({
        "matches": matches,
        "count": matches.len(),
        "truncated": false
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_captures_from_text() {
        let input = json!({
            "text": "id=abc\nid=xyz",
            "pattern": "id=([a-z]+)",
            "group": 1
        });
        let out = run(&input).unwrap();
        assert_eq!(out["count"], 2);
        assert_eq!(out["matches"][0]["value"], "abc");
        assert_eq!(out["matches"][1]["value"], "xyz");
    }
}

