use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::budget::{estimate_tokens, CompactLevel};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactOutput {
    pub command: String,
    pub strategy: String,
    pub level: CompactLevel,
    pub raw_lines: usize,
    pub raw_chars: usize,
    pub raw_tokens: u64,
    pub compact_lines: usize,
    pub compact_chars: usize,
    pub compact_tokens: u64,
    pub tokens_saved: i64,
    pub summary: String,
}

pub fn compact_command_output(command: &str, output: &str, level: CompactLevel) -> CompactOutput {
    let strategy = command_key(command);
    let summary = match strategy.as_str() {
        "git-status" => compact_git_status(output),
        "git-diff" => compact_git_diff(output, level),
        "test-output" => compact_test_output(output, level),
        "search" => compact_search_output(output, level),
        "list" => compact_list_output(output, level),
        "read" => compact_read_output(output, level),
        _ => compact_generic_output(output, level),
    };

    let raw_lines = output.lines().count();
    let raw_chars = output.chars().count();
    let raw_tokens = estimate_tokens(output);
    let compact_lines = summary.lines().count();
    let compact_chars = summary.chars().count();
    let compact_tokens = estimate_tokens(&summary);

    CompactOutput {
        command: command.to_string(),
        strategy,
        level,
        raw_lines,
        raw_chars,
        raw_tokens,
        compact_lines,
        compact_chars,
        compact_tokens,
        tokens_saved: raw_tokens as i64 - compact_tokens as i64,
        summary,
    }
}

fn command_key(command: &str) -> String {
    let lower = command.trim().to_lowercase();
    if lower.starts_with("git status") {
        "git-status".to_string()
    } else if lower.starts_with("git diff") {
        "git-diff".to_string()
    } else if lower.starts_with("cargo test")
        || lower.starts_with("pytest")
        || lower.starts_with("go test")
        || lower.starts_with("npm test")
        || lower.starts_with("pnpm test")
    {
        "test-output".to_string()
    } else if lower.starts_with("rg ")
        || lower.starts_with("grep ")
        || lower.starts_with("ripgrep ")
    {
        "search".to_string()
    } else if lower.starts_with("ls ") || lower == "ls" || lower.starts_with("find ") {
        "list".to_string()
    } else if lower.starts_with("cat ") || lower.starts_with("read ") {
        "read".to_string()
    } else {
        "generic".to_string()
    }
}

fn level_limit(level: CompactLevel, full: usize, compact: usize, aggressive: usize) -> usize {
    match level {
        CompactLevel::Full => full,
        CompactLevel::Compact => compact,
        CompactLevel::Aggressive => aggressive,
    }
}

fn compact_git_status(output: &str) -> String {
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    let mut added = Vec::new();
    let mut renamed = Vec::new();
    let mut untracked = Vec::new();

    for line in output.lines().map(str::trim) {
        if let Some(path) = line.strip_prefix("modified:") {
            modified.push(path.trim().to_string());
        } else if let Some(path) = line.strip_prefix("deleted:") {
            deleted.push(path.trim().to_string());
        } else if let Some(path) = line.strip_prefix("new file:") {
            added.push(path.trim().to_string());
        } else if let Some(path) = line.strip_prefix("renamed:") {
            renamed.push(path.trim().to_string());
        } else if let Some(path) = line.strip_prefix("?? ") {
            untracked.push(path.trim().to_string());
        }
    }

    let mut lines = Vec::new();
    lines.push("Git status summary".to_string());
    push_group(&mut lines, "Modified", &modified, 12);
    push_group(&mut lines, "Added", &added, 12);
    push_group(&mut lines, "Deleted", &deleted, 12);
    push_group(&mut lines, "Renamed", &renamed, 12);
    push_group(&mut lines, "Untracked", &untracked, 12);
    if lines.len() == 1 {
        lines.push(output.lines().take(8).collect::<Vec<_>>().join("\n"));
    }
    lines.join("\n")
}

fn compact_git_diff(output: &str, level: CompactLevel) -> String {
    let mut files = Vec::new();
    let mut current_file = None::<String>;
    let mut current_hunks = Vec::new();
    let mut adds = 0usize;
    let mut dels = 0usize;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            if let Some(file) = current_file.take() {
                files.push((file, current_hunks.clone()));
                current_hunks.clear();
            }
            let file = rest
                .split_whitespace()
                .next()
                .unwrap_or(rest)
                .trim_end_matches(" b/")
                .to_string();
            current_file = Some(file);
        } else if line.starts_with("@@") {
            current_hunks.push(line.to_string());
        } else if line.starts_with('+') && !line.starts_with("+++") {
            adds += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            dels += 1;
        }
    }
    if let Some(file) = current_file.take() {
        files.push((file, current_hunks));
    }

    let file_limit = level_limit(level, 8, 5, 3);
    let hunk_limit = level_limit(level, 3, 2, 1);
    let mut lines = vec![format!(
        "Git diff summary: {} files changed, +{} / -{}",
        files.len(),
        adds,
        dels
    )];
    for (file, hunks) in files.iter().take(file_limit) {
        lines.push(format!("FILE {}", file));
        for hunk in hunks.iter().take(hunk_limit) {
            lines.push(format!("  {}", hunk));
        }
    }
    if files.len() > file_limit {
        lines.push(format!(
            "... {} more files omitted",
            files.len() - file_limit
        ));
    }
    lines.join("\n")
}

fn compact_test_output(output: &str, level: CompactLevel) -> String {
    let mut failures = Vec::new();
    let mut warnings = 0usize;
    let mut errors = 0usize;
    let mut summary = None::<String>;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("test ") && trimmed.contains(" ... FAILED") {
            failures.push(trimmed.to_string());
        } else if trimmed.contains("FAILED") && trimmed.contains("::") {
            failures.push(trimmed.to_string());
        } else if trimmed.starts_with("error:") || trimmed.contains(" error[") {
            errors += 1;
        } else if trimmed.starts_with("warning:") {
            warnings += 1;
        } else if trimmed.contains("test result:")
            || trimmed.contains("passed in")
            || trimmed == "PASS"
        {
            summary = Some(trimmed.to_string());
        }
    }

    let fail_limit = level_limit(level, 10, 6, 3);
    let mut lines = vec!["Test output summary".to_string()];
    if let Some(summary) = summary {
        lines.push(format!("Summary: {}", summary));
    }
    if errors > 0 {
        lines.push(format!("Errors: {}", errors));
    }
    if warnings > 0 {
        lines.push(format!("Warnings: {}", warnings));
    }
    if failures.is_empty() {
        lines.push("Failures: none detected".to_string());
    } else {
        lines.push(format!("Failures: {}", failures.len()));
        for failure in failures.iter().take(fail_limit) {
            lines.push(format!("  {}", failure));
        }
        if failures.len() > fail_limit {
            lines.push(format!(
                "... {} more failures omitted",
                failures.len() - fail_limit
            ));
        }
    }
    lines.join("\n")
}

fn compact_search_output(output: &str, level: CompactLevel) -> String {
    let mut by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in output.lines() {
        let mut parts = line.splitn(3, ':');
        let Some(path) = parts.next() else { continue };
        let Some(line_no) = parts.next() else {
            continue;
        };
        let Some(rest) = parts.next() else { continue };
        by_file
            .entry(path.to_string())
            .or_default()
            .push(format!("{}: {}", line_no, rest.trim()));
    }

    let file_limit = level_limit(level, 8, 5, 3);
    let match_limit = level_limit(level, 4, 2, 1);
    let mut lines = vec![format!(
        "Search summary: {} matches across {} files",
        output.lines().count(),
        by_file.len()
    )];
    for (path, matches) in by_file.iter().take(file_limit) {
        lines.push(format!("FILE {} ({})", path, matches.len()));
        for hit in matches.iter().take(match_limit) {
            lines.push(format!("  {}", hit));
        }
    }
    if by_file.len() > file_limit {
        lines.push(format!(
            "... {} more files omitted",
            by_file.len() - file_limit
        ));
    }
    lines.join("\n")
}

fn compact_list_output(output: &str, level: CompactLevel) -> String {
    let mut entries = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    entries.sort();
    let limit = level_limit(level, 24, 12, 8);
    let mut lines = vec![format!("Listing summary: {} entries", entries.len())];
    for entry in entries.iter().take(limit) {
        lines.push(entry.to_string());
    }
    if entries.len() > limit {
        lines.push(format!(
            "... {} more entries omitted",
            entries.len() - limit
        ));
    }
    lines.join("\n")
}

fn compact_read_output(output: &str, level: CompactLevel) -> String {
    let signature_limit = level_limit(level, 12, 8, 4);
    let excerpt_limit = level_limit(level, 24, 14, 8);
    let mut signatures = Vec::new();
    let mut excerpt = Vec::new();
    for line in output.lines().map(str::trim_end) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if looks_like_signature(trimmed) {
            signatures.push(trimmed.to_string());
        }
        if excerpt.len() < excerpt_limit {
            excerpt.push(trimmed.to_string());
        }
    }

    let mut lines = vec![format!(
        "Read summary: {} lines, ~{} tokens",
        output.lines().count(),
        estimate_tokens(output)
    )];
    if !signatures.is_empty() {
        lines.push("Key signatures:".to_string());
        for signature in signatures.iter().take(signature_limit) {
            lines.push(format!("  {}", signature));
        }
        if signatures.len() > signature_limit {
            lines.push(format!(
                "... {} more signatures omitted",
                signatures.len() - signature_limit
            ));
        }
    }
    lines.push("Excerpt:".to_string());
    lines.extend(excerpt);
    lines.join("\n")
}

fn compact_generic_output(output: &str, level: CompactLevel) -> String {
    let limit = level_limit(level, 28, 16, 8);
    let lines = output
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .take(limit)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return "(empty output)".to_string();
    }
    let mut summary = vec![format!(
        "Generic summary: {} lines, ~{} tokens",
        output.lines().count(),
        estimate_tokens(output)
    )];
    summary.extend(lines.iter().map(|line| line.to_string()));
    if output.lines().count() > limit {
        summary.push("... output truncated".to_string());
    }
    summary.join("\n")
}

fn looks_like_signature(line: &str) -> bool {
    let prefixes = [
        "fn ",
        "pub fn ",
        "async fn ",
        "pub async fn ",
        "struct ",
        "pub struct ",
        "enum ",
        "pub enum ",
        "trait ",
        "pub trait ",
        "impl ",
        "class ",
        "interface ",
        "def ",
        "function ",
    ];
    prefixes.iter().any(|prefix| line.starts_with(prefix))
}

fn push_group(lines: &mut Vec<String>, label: &str, items: &[String], limit: usize) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{}: {}", label, items.len()));
    for item in items.iter().take(limit) {
        lines.push(format!("  {}", item));
    }
    if items.len() > limit {
        lines.push(format!(
            "... {} more {} entries omitted",
            items.len() - limit,
            label.to_lowercase()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_git_status() {
        let output = "modified: src/main.rs\nmodified: Cargo.toml\n?? tests/new.rs\n";
        let compact = compact_command_output("git status", output, CompactLevel::Compact);
        assert_eq!(compact.strategy, "git-status");
        assert!(compact.summary.contains("Modified: 2"));
        assert!(compact.summary.contains("Untracked: 1"));
    }

    #[test]
    fn compacts_test_output() {
        let output = "warning: one\nerror: fail\ntest auth::works ... FAILED\ntest result: FAILED. 9 passed; 1 failed";
        let compact = compact_command_output("cargo test", output, CompactLevel::Compact);
        assert_eq!(compact.strategy, "test-output");
        assert!(compact.summary.contains("Failures: 1"));
        assert!(compact.summary.contains("Errors: 1"));
    }

    #[test]
    fn compacts_search_output() {
        let output = "src/main.rs:10: fn main()\nsrc/main.rs:20: run()\nsrc/lib.rs:5: pub fn run()";
        let compact = compact_command_output("rg run src", output, CompactLevel::Compact);
        assert_eq!(compact.strategy, "search");
        assert!(compact.summary.contains("src/main.rs"));
        assert!(compact.summary.contains("src/lib.rs"));
    }
}
