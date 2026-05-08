pub mod delta_compare;
pub mod csv_slice;
pub mod diff;
pub mod file_stat;
pub mod glob;
pub mod group_count;
pub mod grep;
pub mod artifact_read;
pub mod join_on_key;
pub mod json_extract;
pub mod json_query;
pub mod json_query_v2;
pub mod logs_correlate;
pub mod logs_slice;
pub mod math;
pub mod metrics_summary;
pub mod semantic_search_local;
pub mod table_select;
pub mod table_stats;
pub mod text_extract;
pub mod time_window_filter;
pub mod workflow_outputs_index;
pub mod workflow_step_health;
pub mod workflow_status_summary;

use anyhow::Result;
use serde_json::Value;

pub fn dispatch(tool: &str, input: &Value) -> Result<Value> {
    match tool {
        "etr.delta_compare" => delta_compare::run(input),
        "etr.grep" => grep::run(input),
        "etr.artifact_read" => artifact_read::run(input),
        "etr.group_count" => group_count::run(input),
        "etr.join_on_key" => join_on_key::run(input),
        "etr.json_extract" => json_extract::run(input),
        "etr.json_query" => json_query::run(input),
        "etr.json_query_v2" => json_query_v2::run(input),
        "etr.logs_correlate" => logs_correlate::run(input),
        "etr.logs_slice" => logs_slice::run(input),
        "etr.csv_slice" => csv_slice::run(input),
        "etr.glob" => glob::run(input),
        "etr.file_stat" => file_stat::run(input),
        "etr.math" => math::run(input),
        "etr.metrics_summary" => metrics_summary::run(input),
        "etr.semantic_search_local" => semantic_search_local::run(input),
        "etr.table_select" => table_select::run(input),
        "etr.table_stats" => table_stats::run(input),
        "etr.text_extract" => text_extract::run(input),
        "etr.time_window_filter" => time_window_filter::run(input),
        "etr.workflow_outputs_index" => workflow_outputs_index::run(input),
        "etr.workflow_step_health" => workflow_step_health::run(input),
        "etr.workflow_status_summary" => workflow_status_summary::run(input),
        "etr.diff" => diff::run(input),
        other => anyhow::bail!("Unknown L1 tool: {other}"),
    }
}

pub fn list() -> Vec<crate::types::ToolManifest> {
    vec![
        crate::types::ToolManifest {
            name: "etr.delta_compare".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Compare baseline vs candidate numeric metrics with thresholds".into(),
            input_schema: serde_json::json!({"baseline":"object?","candidate":"object?","baseline_path":"string?","candidate_path":"string?","thresholds":"object?"}),
            output_schema: serde_json::json!({"deltas":"object","violations":"array","violation_count":"integer"}),
        },
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
            name: "etr.artifact_read".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Read artifact files as text or JSON with size guardrails".into(),
            input_schema: serde_json::json!({
                "path": "string",
                "mode": "string (optional: text|json, default text)",
                "max_bytes": "integer (optional, default 100000)"
            }),
            output_schema: serde_json::json!({"path":"string","mode":"string","size_bytes":"integer","truncated":"boolean","text or value":"any"}),
        },
        crate::types::ToolManifest {
            name: "etr.group_count".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Group by field and count occurrences with top-k".into(),
            input_schema: serde_json::json!({"field":"string","rows":"array?","path":"string?","format":"string?","top_k":"integer?"}),
            output_schema: serde_json::json!({"groups":"array","count":"integer"}),
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
            name: "etr.json_query_v2".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Enhanced JSON query with projection, sort, and limit".into(),
            input_schema: serde_json::json!({
                "path": "string (JSON file path)",
                "query": "string (e.g. .items[*], .items[kind=error])",
                "project": "array of string (optional field projection)",
                "sort_by": "string (optional field name)",
                "sort_order": "string (optional: asc|desc, default asc)",
                "limit": "integer (optional)"
            }),
            output_schema: serde_json::json!({"matches":"array","count":"integer"}),
        },
        crate::types::ToolManifest {
            name: "etr.join_on_key".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Join left/right datasets on key fields".into(),
            input_schema: serde_json::json!({"left_rows":"array?","right_rows":"array?","left_path":"string?","right_path":"string?","left_key":"string","right_key":"string","how":"string?"}),
            output_schema: serde_json::json!({"rows":"array","count":"integer"}),
        },
        crate::types::ToolManifest {
            name: "etr.logs_correlate".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Correlate two logs by extracted regex token".into(),
            input_schema: serde_json::json!({"path_a":"string","path_b":"string","pattern":"string","group":"integer?"}),
            output_schema: serde_json::json!({"matches":"array","count":"integer"}),
        },
        crate::types::ToolManifest {
            name: "etr.logs_slice".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Filter log files by level and substring with bounded output".into(),
            input_schema: serde_json::json!({
                "path":"string",
                "level":"string (optional: trace|debug|info|warn|error|fatal)",
                "contains":"string (optional substring)",
                "max_lines":"integer (optional, default 200)"
            }),
            output_schema: serde_json::json!({"path":"string","count":"integer","lines":"array"}),
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
            name: "etr.metrics_summary".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Compute min/max/mean/p50/p95/stddev for numeric columns".into(),
            input_schema: serde_json::json!({"rows":"array?","path":"string?","format":"string?","columns":"array?"}),
            output_schema: serde_json::json!({"metrics":"object"}),
        },
        crate::types::ToolManifest {
            name: "etr.semantic_search_local".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Token-overlap semantic-like search over local text files".into(),
            input_schema: serde_json::json!({
                "query":"string",
                "root":"string (optional, default '.')",
                "pattern":"string (optional glob, default **/*.{md,rs,toml,txt,json,yaml,yml})",
                "limit":"integer (optional, default 10)"
            }),
            output_schema: serde_json::json!({"count":"integer","results":"array"}),
        },
        crate::types::ToolManifest {
            name: "etr.table_select".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Select/filter/order/limit rows from CSV/JSONL".into(),
            input_schema: serde_json::json!({"path":"string","format":"string?","columns":"array?","where":"object?","order_by":"string?","order_desc":"boolean?","limit":"integer?"}),
            output_schema: serde_json::json!({"rows":"array","count":"integer"}),
        },
        crate::types::ToolManifest {
            name: "etr.table_stats".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Compute compact table statistics for CSV/JSONL files".into(),
            input_schema: serde_json::json!({
                "path": "string",
                "format": "string (optional: auto|csv|jsonl, default auto)",
                "max_distinct": "integer (optional, default 20)"
            }),
            output_schema: serde_json::json!({"format":"string","rows":"integer","column_count":"integer","columns":"array"}),
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
            name: "etr.time_window_filter".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Filter rows by RFC3339 timestamp window".into(),
            input_schema: serde_json::json!({"rows":"array?","path":"string?","format":"string?","timestamp_field":"string?","start":"string","end":"string"}),
            output_schema: serde_json::json!({"rows":"array","count":"integer"}),
        },
        crate::types::ToolManifest {
            name: "etr.workflow_status_summary".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Summarize hosted workflow progress JSON into compact status fields".into(),
            input_schema: serde_json::json!({
                "path": "string (JSON file path)"
            }),
            output_schema: serde_json::json!({
                "completed_steps":"integer",
                "total_steps":"integer",
                "running_steps":"array of string",
                "ready_steps":"array of string",
                "pending_approval":"string|null",
                "last_error":"any",
                "outputs_available":"array of string"
            }),
        },
        crate::types::ToolManifest {
            name: "etr.workflow_outputs_index".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Index available workflow outputs from state JSON".into(),
            input_schema: serde_json::json!({"path":"string"}),
            output_schema: serde_json::json!({"outputs":"array","count":"integer"}),
        },
        crate::types::ToolManifest {
            name: "etr.workflow_step_health".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Summarize workflow step failures and running counts".into(),
            input_schema: serde_json::json!({"path":"string"}),
            output_schema: serde_json::json!({"running_steps":"integer","failed_steps":"array","failed_count":"integer"}),
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
