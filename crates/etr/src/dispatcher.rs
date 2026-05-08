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
        let audit_path =
            workspace_root.join(".agent007/runtime/etr_audit.jsonl");
        Self {
            policy: PolicyEngine::new(workspace_root.clone()),
            audit: AuditLog::new(audit_path),
            workspace_root,
        }
    }

    pub fn call(&self, req: EtrCallRequest) -> EtrCallResult {
        let audit_id = format!("etr-{}", uuid::Uuid::new_v4().simple());
        let start = Instant::now();
        let input_size =
            serde_json::to_string(&req.input).map(|s| s.len()).unwrap_or(0);

        if let PolicyResult::Denied(reason) =
            self.policy.check(&req.tool, &req.input)
        {
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

        match l1::dispatch(&req.tool, &req.input) {
            Ok(raw_output) => {
                let output_size = serde_json::to_string(&raw_output)
                    .map(|s| s.len())
                    .unwrap_or(0);
                let (output, truncated) = if req.compact {
                    let (v, t) = Compactor::compact_json(raw_output, None);
                    (v, Some(t))
                } else {
                    (raw_output, None)
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
                self.audit.write(
                    &req.tool,
                    &audit_id,
                    "error",
                    latency_ms,
                    input_size,
                    0,
                );
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
            description: "List all available ETR tools with their input/output schemas"
                .into(),
            input_schema: serde_json::json!({
                "layer": "string (optional: 'l1', 'l2', 'l3', 'all')"
            }),
            output_schema: serde_json::json!({"tools": "array of ToolManifest"}),
        });
        tools
    }
}
