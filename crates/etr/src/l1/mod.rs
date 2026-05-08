pub mod csv_slice;
pub mod diff;
pub mod file_stat;
pub mod glob;
pub mod grep;
pub mod json_extract;
pub mod json_query;
pub mod math;
pub mod text_extract;

use anyhow::Result;
use serde_json::Value;

pub fn dispatch(tool: &str, input: &Value) -> Result<Value> {
    match tool {
        "etr.grep" => grep::run(input),
        "etr.json_extract" => json_extract::run(input),
        "etr.json_query" => json_query::run(input),
        "etr.csv_slice" => csv_slice::run(input),
        "etr.glob" => glob::run(input),
        "etr.file_stat" => file_stat::run(input),
        "etr.math" => math::run(input),
        "etr.text_extract" => text_extract::run(input),
        "etr.diff" => diff::run(input),
        other => anyhow::bail!("Unknown L1 tool: {other}"),
    }
}

pub fn list() -> Vec<crate::types::ToolManifest> {
    vec![
        crate::types::ToolManifest {
            name: "etr.grep".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Search files for a regex pattern".into(),
            input_schema: serde_json::json!({
                "pattern": "string (regex)",
                "path": "string (file or directory)",
                "context_lines": "integer (optional, default 0)"
            }),
            output_schema: serde_json::json!({"matches": "array", "count": "integer"}),
        },
        crate::types::ToolManifest {
            name: "etr.json_extract".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Extract a value from a JSON file using a dot-path (e.g. .results.0.score)".into(),
            input_schema: serde_json::json!({
                "path": "string (JSON file path)",
                "jq_path": "string (dot-separated path, e.g. .field.subfield)"
            }),
            output_schema: serde_json::json!({"value": "any"}),
        },
        crate::types::ToolManifest {
            name: "etr.json_query".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Query JSON with dot-path + selectors ([index], [*], [field=value])".into(),
            input_schema: serde_json::json!({
                "path": "string (JSON file path)",
                "query": "string (e.g. .results[*].score, .items[kind=error].id)"
            }),
            output_schema: serde_json::json!({"matches": "array", "count": "integer"}),
        },
        crate::types::ToolManifest {
            name: "etr.csv_slice".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Slice rows from a CSV file with optional column selection and row limit".into(),
            input_schema: serde_json::json!({
                "path": "string",
                "columns": "array of string (optional)",
                "limit": "integer (optional, default 50)"
            }),
            output_schema: serde_json::json!({"rows": "array", "total_rows": "integer", "truncated": "boolean"}),
        },
        crate::types::ToolManifest {
            name: "etr.glob".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Find files matching a glob pattern".into(),
            input_schema: serde_json::json!({
                "pattern": "string",
                "root": "string (optional, default '.')"
            }),
            output_schema: serde_json::json!({"paths": "array of string"}),
        },
        crate::types::ToolManifest {
            name: "etr.file_stat".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Get file metadata (size, mtime, exists)".into(),
            input_schema: serde_json::json!({
                "path": "string"
            }),
            output_schema: serde_json::json!({"exists": "boolean", "size_bytes": "integer", "mtime": "string (RFC3339)"}),
        },
        crate::types::ToolManifest {
            name: "etr.math".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Evaluate a math expression (e.g. '2 + 3 * 4', '2^8', 'math::sqrt(16)', 'math::log(100, 10)')".into(),
            input_schema: serde_json::json!({
                "expression": "string"
            }),
            output_schema: serde_json::json!({"result": "number or string"}),
        },
        crate::types::ToolManifest {
            name: "etr.text_extract".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Extract regex matches from text or file with optional capture group".into(),
            input_schema: serde_json::json!({
                "pattern": "string (regex)",
                "path": "string (optional; file path)",
                "text": "string (optional; inline text)",
                "group": "integer (optional, default 0)",
                "max_matches": "integer (optional, default 100)"
            }),
            output_schema: serde_json::json!({"matches": "array", "count": "integer", "truncated": "boolean"}),
        },
        crate::types::ToolManifest {
            name: "etr.diff".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Compute a unified diff between two text files".into(),
            input_schema: serde_json::json!({
                "path_a": "string",
                "path_b": "string"
            }),
            output_schema: serde_json::json!({"unified_diff": "string", "changed_lines": "integer"}),
        },
    ]
}
