use serde_json::json;
use std::path::PathBuf;

pub struct AuditLog {
    pub path: PathBuf,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn write(
        &self,
        tool: &str,
        audit_id: &str,
        status: &str,
        latency_ms: u64,
        input_size: usize,
        output_size: usize,
    ) {
        use std::io::Write;
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let entry = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "audit_id": audit_id,
            "tool": tool,
            "status": status,
            "latency_ms": latency_ms,
            "input_bytes": input_size,
            "output_bytes": output_size,
        });
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(f, "{}", serde_json::to_string(&entry).unwrap_or_default());
        }
    }
}
