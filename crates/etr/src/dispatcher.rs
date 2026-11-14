use std::path::PathBuf;
use std::time::Instant;

use crate::audit::AuditLog;
use crate::compactor::Compactor;
use crate::l1;
use crate::policy::{PolicyEngine, PolicyResult};
use crate::types::{EtrCallRequest, EtrCallResult, EtrStatus, ToolManifest};

pub struct EtrDispatcher {
    pub policy: PolicyEngine,
    pub audit: AuditLog,
    pub workspace_root: PathBuf,
}

impl EtrDispatcher {
    pub fn new(workspace_root: PathBuf) -> Self {
        let audit_path = workspace_root.join(".agent007/runtime/etr_audit.jsonl");
        Self {
            policy: PolicyEngine::new(workspace_root.clone()),
            audit: AuditLog::new(audit_path),
            workspace_root,
        }
    }

    pub fn call(&self, req: EtrCallRequest) -> EtrCallResult {
        let audit_id = format!("etr-{}", uuid::Uuid::new_v4().simple());
        let start = Instant::now();
        let input_size = serde_json::to_string(&req.input)
            .map(|s| s.len())
            .unwrap_or(0);

        if input_size > 1024 * 1024 || (!req.input.is_object() && req.tool != "etr.list") {
            let latency_ms = start.elapsed().as_millis() as u64;
            self.audit
                .write(&req.tool, &audit_id, "denied", latency_ms, input_size, 0);
            return EtrCallResult {
                tool: req.tool,
                status: EtrStatus::Denied,
                output: serde_json::json!({}),
                audit_id,
                error: Some("Tool input must be an object within the 1 MiB budget".into()),
                truncated: None,
                latency_ms,
            };
        }
        if req.tool == "etr.policy_check" {
            let tool = req.input["tool"].as_str().unwrap_or("");
            let input = req
                .input
                .get("input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let decision = match self.policy.check(tool, &input) {
                PolicyResult::Allowed => serde_json::json!({
                    "allowed": true,
                    "reason": null
                }),
                PolicyResult::Denied(reason) => serde_json::json!({
                    "allowed": false,
                    "reason": reason
                }),
            };
            let latency_ms = start.elapsed().as_millis() as u64;
            self.audit
                .write(&req.tool, &audit_id, "ok", latency_ms, input_size, 0);
            return EtrCallResult {
                tool: req.tool,
                status: EtrStatus::Ok,
                output: decision,
                audit_id,
                error: None,
                truncated: None,
                latency_ms,
            };
        }

        if let PolicyResult::Denied(reason) = self.policy.check(&req.tool, &req.input) {
            let latency_ms = start.elapsed().as_millis() as u64;
            self.audit
                .write(&req.tool, &audit_id, "denied", latency_ms, input_size, 0);
            return EtrCallResult {
                tool: req.tool,
                status: EtrStatus::Denied,
                output: serde_json::json!({}),
                audit_id,
                error: Some(reason),
                truncated: None,
                latency_ms,
            };
        }

        if req.tool == "etr.list" {
            let manifests = l1::list();
            let latency_ms = start.elapsed().as_millis() as u64;
            let output = serde_json::json!({ "tools": manifests });
            self.audit
                .write(&req.tool, &audit_id, "ok", latency_ms, input_size, 0);
            return EtrCallResult {
                tool: req.tool,
                status: EtrStatus::Ok,
                output,
                audit_id,
                error: None,
                truncated: None,
                latency_ms,
            };
        }

        let input = match self.policy.prepare(&req.input) {
            Ok(mut v) => {
                if l1::list()
                    .iter()
                    .any(|m| m.name == req.tool && m.input_schema.get("root").is_some())
                    && v.get("root").is_none()
                {
                    v["root"] = self.workspace_root.to_string_lossy().to_string().into();
                }
                v
            }
            Err(e) => {
                return EtrCallResult {
                    tool: req.tool,
                    status: EtrStatus::Denied,
                    output: serde_json::json!({}),
                    audit_id,
                    error: Some(e),
                    truncated: None,
                    latency_ms: 0,
                }
            }
        };
        match l1::dispatch(&req.tool, &input) {
            Ok(raw_output) => {
                let output_size = serde_json::to_string(&raw_output)
                    .map(|s| s.len())
                    .unwrap_or(0);
                let work_truncated = raw_output
                    .get("truncated")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let (output, truncated) = if req.compact {
                    let (v, t) = Compactor::compact_json(raw_output, None);
                    (v, Some(t || work_truncated))
                } else {
                    (raw_output, Some(work_truncated))
                };
                let latency_ms = start.elapsed().as_millis() as u64;
                self.audit.write(
                    &req.tool,
                    &audit_id,
                    "ok",
                    latency_ms,
                    input_size,
                    output_size,
                );
                EtrCallResult {
                    tool: req.tool,
                    status: EtrStatus::Ok,
                    output,
                    audit_id,
                    error: None,
                    truncated,
                    latency_ms,
                }
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                self.audit
                    .write(&req.tool, &audit_id, "error", latency_ms, input_size, 0);
                EtrCallResult {
                    tool: req.tool,
                    status: EtrStatus::Error,
                    output: serde_json::json!({}),
                    audit_id,
                    error: Some(e.to_string()),
                    truncated: None,
                    latency_ms,
                }
            }
        }
    }

    pub fn list_tools(&self) -> Vec<ToolManifest> {
        let mut tools = l1::list();
        tools.push(ToolManifest {
            name: "etr.list".into(),
            layer: crate::types::ToolLayer::L1,
            description: "List all available ETR tools with their input/output schemas".into(),
            input_schema: serde_json::json!({
                "layer": "string (optional: 'l1', 'l2', 'l3', 'all')"
            }),
            output_schema: serde_json::json!({"tools": "array of ToolManifest"}),
        });
        tools.push(ToolManifest {
            name: "etr.policy_check".into(),
            layer: crate::types::ToolLayer::L1,
            description: "Evaluate whether a tool call would be allowed by ETR policy".into(),
            input_schema: serde_json::json!({
                "tool":"string (target tool name)",
                "input":"object (target tool input)"
            }),
            output_schema: serde_json::json!({
                "allowed":"boolean",
                "reason":"string|null"
            }),
        });
        tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_relative_files_and_default_root_remain_usable() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("note.txt"), "needle").unwrap();
        let dispatcher = EtrDispatcher::new(d.path().to_path_buf());
        for (tool, input) in [
            (
                "etr.grep",
                serde_json::json!({"path":"note.txt","pattern":"needle"}),
            ),
            ("etr.glob", serde_json::json!({"pattern":"*.txt"})),
        ] {
            let r = dispatcher.call(EtrCallRequest {
                tool: tool.into(),
                input,
                compact: true,
            });
            assert!(matches!(r.status, EtrStatus::Ok), "{:?}", r.error);
        }
    }
    #[test]
    fn malformed_and_oversized_inputs_are_denied_and_audited() {
        let d = tempfile::tempdir().unwrap();
        let dispatcher = EtrDispatcher::new(d.path().to_path_buf());
        for input in [
            serde_json::json!([]),
            serde_json::json!(null),
            serde_json::json!({"text":"x".repeat(1024*1024+1)}),
        ] {
            let r = dispatcher.call(EtrCallRequest {
                tool: "etr.glob".into(),
                input,
                compact: true,
            });
            assert!(matches!(r.status, EtrStatus::Denied));
        }
    }
    #[test]
    fn policy_check_reports_denied_for_path_traversal() {
        let base = std::env::temp_dir().join(format!("etr-policy-{}", uuid::Uuid::new_v4()));
        let ws = base.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(base.join("outside.txt"), "x").unwrap();
        let d = EtrDispatcher::new(ws);
        let res = d.call(EtrCallRequest {
            tool: "etr.policy_check".to_string(),
            input: serde_json::json!({
                "tool":"etr.file_stat",
                "input":{"path":"../outside.txt"}
            }),
            compact: false,
        });
        assert_eq!(res.status, EtrStatus::Ok);
        assert_eq!(res.output["allowed"], false);
        let _ = std::fs::remove_dir_all(base);
    }
}
