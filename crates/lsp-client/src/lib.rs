use std::path::{Path, PathBuf};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Default)]
pub struct LspContext {
    pub diagnostics: Vec<Diagnostic>,
    pub symbols: Vec<Symbol>,
}

impl LspContext {
    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty() && self.symbols.is_empty()
    }

    /// Format as a concise text block for LLM prompt injection.
    pub fn to_prompt_string(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        if !self.diagnostics.is_empty() {
            out.push_str("## LSP Diagnostics\n");
            for d in &self.diagnostics {
                out.push_str(&format!(
                    "- [{}] {}:{}:{} — {}\n",
                    d.severity.to_uppercase(), d.file, d.line, d.col, d.message
                ));
            }
        }
        if !self.symbols.is_empty() {
            out.push_str("\n## Symbols\n");
            for s in &self.symbols {
                out.push_str(&format!("- {} ({}) at {}:{}\n", s.name, s.kind, s.file, s.line));
            }
        }
        out
    }
}

pub struct LspClient {
    server_cmd: String,
}

impl LspClient {
    pub fn new(server_cmd: &str) -> Self {
        Self { server_cmd: server_cmd.to_string() }
    }

    /// Detect which LSP server to use for the given directory.
    /// Returns (language, server_command) or None if not detected.
    pub fn detect_language(dir: &Path) -> Option<(&'static str, &'static str)> {
        if dir.join("Cargo.toml").exists() {
            return Some(("rust", "rust-analyzer"));
        }
        if dir.join("package.json").exists() {
            return Some(("typescript", "typescript-language-server --stdio"));
        }
        if dir.join("go.mod").exists() {
            return Some(("go", "gopls"));
        }
        if dir.join("pyproject.toml").exists() || dir.join("setup.py").exists() {
            return Some(("python", "pyright --stdio"));
        }
        None
    }

    /// Query the LSP server for diagnostics and symbols for the given files.
    /// Spawns the server, sends requests, collects results, kills the server.
    /// Returns empty LspContext on any error (non-fatal by design).
    pub async fn query(&self, root: &Path, _files: &[PathBuf]) -> Result<LspContext> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::process::Command;

        let parts: Vec<&str> = self.server_cmd.splitn(2, ' ').collect();
        let (cmd, args): (&str, Vec<&str>) = if parts.len() > 1 {
            (parts[0], parts[1].split_whitespace().collect())
        } else {
            (parts[0], vec![])
        };

        let mut child = Command::new(cmd)
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let root_uri = format!("file://{}", root.display());

        // Send initialize request
        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {
                    "textDocument": {
                        "publishDiagnostics": { "relatedInformation": false }
                    },
                    "workspace": {
                        "symbol": { "dynamicRegistration": false }
                    }
                },
                "workspaceFolders": [{ "uri": root_uri, "name": "workspace" }]
            }
        });

        let msg = format_lsp_message(&init_req.to_string());
        let mut stdin = stdin;
        stdin.write_all(msg.as_bytes()).await?;

        // Send initialized notification
        let initialized = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        });
        stdin.write_all(format_lsp_message(&initialized.to_string()).as_bytes()).await?;

        // Send workspace/symbol request
        let sym_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "workspace/symbol",
            "params": { "query": "" }
        });
        stdin.write_all(format_lsp_message(&sym_req.to_string()).as_bytes()).await?;
        stdin.flush().await?;

        // Read responses with a timeout
        let mut reader = BufReader::new(stdout);
        let mut context = LspContext::default();
        let read_timeout = std::time::Duration::from_secs(10);

        let read_task = async {
            let mut responses_read = 0;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).await? == 0 { break; }
                if !header.starts_with("Content-Length:") { continue; }
                let len: usize = header.trim_start_matches("Content-Length:").trim().parse()?;
                // skip blank line
                let mut blank = String::new();
                reader.read_line(&mut blank).await?;
                // read body
                let mut body = vec![0u8; len];
                tokio::io::AsyncReadExt::read_exact(&mut reader, &mut body).await?;
                let body_str = String::from_utf8_lossy(&body);
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body_str) {
                    parse_lsp_response(&v, &mut context);
                    if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                        if id == 2 { responses_read += 1; }
                    }
                    if responses_read >= 1 { break; }
                }
            }
            Ok::<(), anyhow::Error>(())
        };

        let _ = tokio::time::timeout(read_timeout, read_task).await;

        let _ = child.kill().await;
        Ok(context)
    }
}

fn format_lsp_message(content: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{}", content.len(), content)
}

fn parse_lsp_response(v: &serde_json::Value, ctx: &mut LspContext) {
    // workspace/symbol response
    if let Some(result) = v.get("result") {
        if let Some(symbols) = result.as_array() {
            for sym in symbols.iter().take(50) {
                let name = sym["name"].as_str().unwrap_or("").to_string();
                let kind = symbol_kind_name(sym["kind"].as_u64().unwrap_or(0));
                let file = sym["location"]["uri"].as_str()
                    .unwrap_or("")
                    .trim_start_matches("file://")
                    .to_string();
                let line = sym["location"]["range"]["start"]["line"].as_u64().unwrap_or(0) as u32 + 1;
                if !name.is_empty() {
                    ctx.symbols.push(Symbol { name, kind, file, line });
                }
            }
        }
    }
    // textDocument/publishDiagnostics notification
    if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
        if method == "textDocument/publishDiagnostics" {
            if let Some(params) = v.get("params") {
                let file = params["uri"].as_str()
                    .unwrap_or("")
                    .trim_start_matches("file://")
                    .to_string();
                if let Some(diags) = params["diagnostics"].as_array() {
                    for d in diags {
                        let severity = match d["severity"].as_u64() {
                            Some(1) => "error",
                            Some(2) => "warning",
                            Some(3) => "info",
                            _ => "hint",
                        };
                        ctx.diagnostics.push(Diagnostic {
                            file: file.clone(),
                            line: d["range"]["start"]["line"].as_u64().unwrap_or(0) as u32 + 1,
                            col: d["range"]["start"]["character"].as_u64().unwrap_or(0) as u32 + 1,
                            severity: severity.to_string(),
                            message: d["message"].as_str().unwrap_or("").to_string(),
                        });
                    }
                }
            }
        }
    }
}

fn symbol_kind_name(kind: u64) -> String {
    match kind {
        1 => "file", 2 => "module", 3 => "namespace", 4 => "package",
        5 => "class", 6 => "method", 7 => "property", 8 => "field",
        9 => "constructor", 10 => "enum", 11 => "interface", 12 => "function",
        13 => "variable", 14 => "constant", 15 => "string", 16 => "number",
        17 => "boolean", 18 => "array", 19 => "object", 20 => "key",
        21 => "null", 22 => "enum_member", 23 => "struct", 24 => "event",
        25 => "operator", 26 => "type_parameter",
        _ => "unknown",
    }.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_context_to_prompt_string_empty() {
        let ctx = LspContext::default();
        assert!(ctx.to_prompt_string().is_empty());
    }

    #[test]
    fn lsp_context_to_prompt_string_with_diagnostic() {
        let ctx = LspContext {
            diagnostics: vec![Diagnostic {
                file: "src/main.rs".to_string(),
                line: 10,
                col: 5,
                severity: "error".to_string(),
                message: "cannot find value".to_string(),
            }],
            symbols: vec![],
        };
        let s = ctx.to_prompt_string();
        assert!(s.contains("ERROR"));
        assert!(s.contains("src/main.rs"));
        assert!(s.contains("cannot find value"));
    }

    #[test]
    fn detect_language_rust() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let result = LspClient::detect_language(dir.path());
        assert_eq!(result, Some(("rust", "rust-analyzer")));
    }

    #[test]
    fn detect_language_typescript() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        let result = LspClient::detect_language(dir.path());
        assert_eq!(result, Some(("typescript", "typescript-language-server --stdio")));
    }

    #[test]
    fn detect_language_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(LspClient::detect_language(dir.path()), None);
    }
}
