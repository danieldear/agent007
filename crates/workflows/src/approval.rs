use serde::{Deserialize, Serialize};
use std::io::Write;
use crate::error::WorkflowError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalDecisionKind {
    Approve,
    Deny,
    Edit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalDecision {
    pub decision: ApprovalDecisionKind,
    pub content: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum ApprovalResponse {
    Approve,
    Deny,
    Edit,
}

impl ApprovalResponse {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "y" | "yes" => ApprovalResponse::Approve,
            "n" | "no" => ApprovalResponse::Deny,
            "e" | "edit" => ApprovalResponse::Edit,
            _ => ApprovalResponse::Deny,
        }
    }
}

pub struct ApprovalGate;

impl ApprovalGate {
    /// Present the approval gate to the user via stderr/stdin.
    /// Returns a structured approval decision, including optional edited content.
    pub async fn prompt(step_id: &str, content: &str) -> Result<ApprovalDecision, WorkflowError> {
        eprintln!("\n[APPROVAL REQUIRED] Step: {}", step_id);
        eprintln!("Output:\n{}\n", content);
        eprint!("Approve? [y/n/edit]: ");
        std::io::stderr().flush().ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)
            .map_err(WorkflowError::Io)?;

        match ApprovalResponse::parse(&input) {
            ApprovalResponse::Approve => Ok(ApprovalDecision {
                decision: ApprovalDecisionKind::Approve,
                content: Some(content.to_string()),
            }),
            ApprovalResponse::Deny => Ok(ApprovalDecision {
                decision: ApprovalDecisionKind::Deny,
                content: None,
            }),
            ApprovalResponse::Edit => {
                let editor = std::env::var("EDITOR").ok();
                let edited = open_in_editor(content, editor.as_deref()).await.map_err(|e| {
                    WorkflowError::StepFailed {
                        id: step_id.to_string(),
                        reason: format!("editor failed: {}", e),
                    }
                })?;
                Ok(ApprovalDecision {
                    decision: ApprovalDecisionKind::Edit,
                    content: Some(edited),
                })
            }
        }
    }
}

/// Write `content` to a tempfile, open `$EDITOR` (or the provided override), and return
/// the file contents after the editor exits.
pub async fn open_in_editor(content: &str, editor: Option<&str>) -> std::io::Result<String> {
    let editor_cmd = editor
        .map(|s| s.to_string())
        .or_else(|| std::env::var("EDITOR").ok())
        .unwrap_or_else(|| "vi".to_string());

    let mut tmpfile = tempfile::Builder::new()
        .suffix(".txt")
        .tempfile()?;
    tmpfile.write_all(content.as_bytes())?;
    tmpfile.flush()?;

    let path = tmpfile.path().to_owned();

    // Spawn editor as a blocking process (uses tokio::task::spawn_blocking)
    let path_clone = path.clone();
    let editor_clone = editor_cmd.clone();
    tokio::task::spawn_blocking(move || {
        std::process::Command::new(&editor_clone)
            .arg(&path_clone)
            .status()
    }).await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    std::fs::read_to_string(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_y_approves() {
        assert_eq!(ApprovalResponse::parse("y"), ApprovalResponse::Approve);
        assert_eq!(ApprovalResponse::parse("Y"), ApprovalResponse::Approve);
        assert_eq!(ApprovalResponse::parse("yes"), ApprovalResponse::Approve);
    }

    #[test]
    fn parse_n_denies() {
        assert_eq!(ApprovalResponse::parse("n"), ApprovalResponse::Deny);
        assert_eq!(ApprovalResponse::parse("N"), ApprovalResponse::Deny);
        assert_eq!(ApprovalResponse::parse("no"), ApprovalResponse::Deny);
    }

    #[test]
    fn parse_edit_returns_edit() {
        assert_eq!(ApprovalResponse::parse("edit"), ApprovalResponse::Edit);
        assert_eq!(ApprovalResponse::parse("e"), ApprovalResponse::Edit);
    }

    #[test]
    fn parse_unknown_defaults_to_deny() {
        assert_eq!(ApprovalResponse::parse("maybe"), ApprovalResponse::Deny);
        assert_eq!(ApprovalResponse::parse(""), ApprovalResponse::Deny);
    }

    #[tokio::test]
    async fn open_editor_returns_original_content_when_editor_is_true() {
        // `true` is a Unix command that exits 0 without modifying files.
        // The tempfile content should remain unchanged → same as original.
        let content = "original output";
        let result = open_in_editor(content, Some("true")).await;
        assert!(result.is_ok());
        // With `true` as editor the file is untouched, so content is returned as-is.
        assert_eq!(result.unwrap(), content);
    }
}
