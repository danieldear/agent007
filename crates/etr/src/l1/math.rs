use anyhow::Context;
use serde_json::{json, Value};

pub fn run(input: &Value) -> Result<Value, anyhow::Error> {
    let expression = input["expression"].as_str().context("expression required")?;
    let result = evalexpr::eval(expression)
        .context(format!("failed to evaluate: {expression}"))?;
    let json_result = match result {
        evalexpr::Value::Float(f) => json!(f),
        evalexpr::Value::Int(i) => json!(i),
        evalexpr::Value::Boolean(b) => json!(b),
        evalexpr::Value::String(s) => json!(s),
        evalexpr::Value::Tuple(t) => json!(format!("{:?}", t)),
        evalexpr::Value::Empty => json!(null),
    };
    Ok(json!({ "result": json_result, "expression": expression }))
}
