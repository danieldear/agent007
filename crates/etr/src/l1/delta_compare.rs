use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

pub fn run(input: &Value) -> Result<Value> {
    let baseline = load_obj(input.get("baseline"), input.get("baseline_path"))?;
    let candidate = load_obj(input.get("candidate"), input.get("candidate_path"))?;
    let thresholds = input["thresholds"].as_object().cloned().unwrap_or_default();

    let mut deltas = Map::new();
    let mut violations = Vec::new();
    for (k, b) in baseline {
        let Some(c) = candidate.get(&k).and_then(to_f64) else {
            continue;
        };
        let Some(bv) = to_f64(&b) else {
            continue;
        };
        let delta = c - bv;
        deltas.insert(k.clone(), Value::from(delta));
        if let Some(t) = thresholds.get(&k).and_then(to_f64) {
            if delta.abs() > t {
                violations.push(json!({"metric":k,"delta":delta,"threshold":t}));
            }
        }
    }
    Ok(json!({"deltas": deltas, "violations": violations, "violation_count": violations.len()}))
}

fn load_obj(inline: Option<&Value>, path: Option<&Value>) -> Result<Map<String, Value>> {
    if let Some(v) = inline.and_then(Value::as_object) {
        return Ok(v.clone());
    }
    let p = path
        .and_then(Value::as_str)
        .context("baseline/candidate object or path required")?;
    let txt = std::fs::read_to_string(p)?;
    let v: Value = serde_json::from_str(&txt)?;
    Ok(v.as_object().cloned().unwrap_or_default())
}

fn to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_with_thresholds() {
        let out = run(&json!({
            "baseline":{"x":10},
            "candidate":{"x":14},
            "thresholds":{"x":3}
        }))
        .unwrap();
        assert_eq!(out["violation_count"], 1);
    }
}
