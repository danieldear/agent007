// crates/core/src/tool_executor.rs
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use agent007_mcp::{McpClient, ToolDef};
use agent007_zones::{AuditEntry, AuditLogger, FileOp, ZoneChecker, ZoneViolation};

use crate::dispatcher::Dispatcher;
use crate::error::CoreError;
use crate::events::{AgentEvent, ToolCall};
use crate::types::AgentId;

/// ToolExecutor enforces zone rules and brokers downstream tool calls.
pub struct ToolExecutor {
    zone_checker: Option<Arc<ZoneChecker>>,
    audit_logger: Option<Arc<AuditLogger>>,
    dispatcher: Option<Arc<dyn Dispatcher>>,
    mcp_client: Option<Arc<AsyncMutex<McpClient>>>,
    agent_name: String,
}

impl ToolExecutor {
    pub fn new(agent_name: impl Into<String>) -> Self {
        Self {
            zone_checker: None,
            audit_logger: None,
            dispatcher: None,
            mcp_client: None,
            agent_name: agent_name.into(),
        }
    }

    pub fn with_zone_checker(mut self, checker: Arc<ZoneChecker>) -> Self {
        self.zone_checker = Some(checker);
        self
    }

    pub fn with_audit_logger(mut self, logger: Arc<AuditLogger>) -> Self {
        self.audit_logger = Some(logger);
        self
    }

    pub fn with_dispatcher(mut self, dispatcher: Arc<dyn Dispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    pub fn with_mcp_client(mut self, client: Arc<AsyncMutex<McpClient>>) -> Self {
        self.mcp_client = Some(client);
        self
    }

    /// Check whether `op` on `path` is permitted.
    /// Writes an audit entry (if a logger is configured) regardless of outcome.
    /// Returns `Ok(())` if allowed, `Err(ZoneViolation)` if blocked.
    pub fn check_zone(&self, path: &Path, op: FileOp) -> Result<(), ZoneViolation> {
        let checker = match &self.zone_checker {
            Some(c) => c,
            None => return Ok(()), // no checker → unrestricted by default
        };

        let result = checker.check(path, op);
        let zone = checker.zone_for(path);
        let allowed = result.is_ok();

        if let Some(logger) = &self.audit_logger {
            let entry = AuditEntry::now(
                &self.agent_name,
                op.as_str(),
                path.to_string_lossy().as_ref(),
                zone.as_str(),
                allowed,
            );
            // Best-effort logging — do not propagate IO errors to callers.
            let _ = logger.log(&entry);
        }

        result
    }

    pub async fn list_mcp_tools(&self) -> Result<Vec<ToolDef>, CoreError> {
        let client = self
            .mcp_client
            .as_ref()
            .ok_or_else(|| CoreError::NotConfigured("MCP client not configured".to_string()))?;
        let locked = client.lock().await;
        locked.list_tools().await.map_err(CoreError::from)
    }

    pub async fn call_mcp_tool(
        &self,
        agent_id: &AgentId,
        tool_name: &str,
        args: Value,
    ) -> Result<Value, CoreError> {
        if let Some(dispatcher) = &self.dispatcher {
            dispatcher
                .publish(AgentEvent::ToolCall {
                    agent_id: agent_id.clone(),
                    tool: ToolCall {
                        name: tool_name.to_string(),
                        args: args.clone(),
                    },
                })
                .await?;
        }

        let client = self
            .mcp_client
            .as_ref()
            .ok_or_else(|| CoreError::NotConfigured("MCP client not configured".to_string()))?;
        let locked = client.lock().await;
        let start = Instant::now();
        let result = locked.call_tool(tool_name, args.clone()).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        if let Some(dispatcher) = &self.dispatcher {
            let (success, error) = match &result {
                Ok(_) => (true, None),
                Err(e) => (false, Some(e.to_string())),
            };
            let _ = dispatcher
                .publish(AgentEvent::ToolCallResult {
                    agent_id: agent_id.clone(),
                    tool: ToolCall {
                        name: tool_name.to_string(),
                        args,
                    },
                    success,
                    error,
                    duration_ms: Some(duration_ms),
                })
                .await;
        }

        result.map_err(CoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_zones::{ZoneConfig, ZoneLevel};
    use tempfile::TempDir;

    fn make_executor(dir: &TempDir) -> ToolExecutor {
        let config = ZoneConfig {
            forbidden: vec!["secrets/".to_string(), ".env".to_string()],
            readonly: vec!["src/auth/".to_string()],
            sensitive: vec!["src/crypto/".to_string()],
            unrestricted: vec!["src/".to_string()],
        };
        let checker = Arc::new(ZoneChecker::new(&config).unwrap());
        let log_path = dir.path().join("audit").join("audit.log");
        let logger = Arc::new(AuditLogger::new(&log_path));

        ToolExecutor::new("TestAgent")
            .with_zone_checker(checker)
            .with_audit_logger(logger)
    }

    #[test]
    fn check_zone_allows_read_on_unrestricted() {
        let dir = TempDir::new().unwrap();
        let ex = make_executor(&dir);
        assert!(ex
            .check_zone(Path::new("src/utils.rs"), FileOp::Read)
            .is_ok());
    }

    #[test]
    fn check_zone_blocks_read_on_forbidden() {
        let dir = TempDir::new().unwrap();
        let ex = make_executor(&dir);
        let result = ex.check_zone(Path::new("secrets/token"), FileOp::Read);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().zone, ZoneLevel::Forbidden);
    }

    #[test]
    fn check_zone_allows_read_but_blocks_write_on_readonly() {
        let dir = TempDir::new().unwrap();
        let ex = make_executor(&dir);
        assert!(ex
            .check_zone(Path::new("src/auth/login.rs"), FileOp::Read)
            .is_ok());
        assert!(ex
            .check_zone(Path::new("src/auth/login.rs"), FileOp::Write)
            .is_err());
    }

    #[test]
    fn check_zone_writes_audit_log_on_violation() {
        let dir = TempDir::new().unwrap();
        let ex = make_executor(&dir);
        let _ = ex.check_zone(Path::new("secrets/token"), FileOp::Read);

        let log_path = dir.path().join("audit").join("audit.log");
        let logger = AuditLogger::new(&log_path);
        let lines = logger.read_lines().unwrap();
        assert_eq!(lines.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(parsed["allowed"], false);
        assert_eq!(parsed["blocked"], true);
        assert_eq!(parsed["agent"], "TestAgent");
    }

    #[test]
    fn check_zone_writes_audit_log_on_allowed_access() {
        let dir = TempDir::new().unwrap();
        let ex = make_executor(&dir);
        let _ = ex.check_zone(Path::new("src/utils.rs"), FileOp::Write);

        let log_path = dir.path().join("audit").join("audit.log");
        let logger = AuditLogger::new(&log_path);
        let lines = logger.read_lines().unwrap();
        assert_eq!(lines.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(parsed["allowed"], true);
        assert!(parsed.get("blocked").is_none());
    }

    #[test]
    fn check_zone_without_checker_always_allows() {
        let ex = ToolExecutor::new("Bare");
        // No checker attached — all ops pass
        assert!(ex
            .check_zone(Path::new("secrets/very_sensitive"), FileOp::Delete)
            .is_ok());
    }

    #[tokio::test]
    async fn list_mcp_tools_without_client_returns_error() {
        let ex = ToolExecutor::new("Bare");
        let result = ex.list_mcp_tools().await;
        assert!(matches!(result, Err(CoreError::NotConfigured(_))));
    }
}
