use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::budget::{estimate_tokens, BudgetEstimate, CompactLevel, TokenBudget};
use crate::compact::compact_command_output;
use crate::error::CoreError;
use crate::repo_brain::{RepoBrain, RepoBrainBuilder};
use crate::run_store::{RunMetadata, RunStore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFile {
    pub path: String,
    pub score: i32,
    pub reason: String,
    pub excerpt: String,
    pub raw_tokens: u64,
    pub excerpt_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMemoryNote {
    pub key: String,
    pub excerpt: String,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBundle {
    pub task: String,
    pub budget: TokenBudget,
    pub recommended_level: CompactLevel,
    pub repo_brain: RepoBrain,
    pub relevant_files: Vec<ContextFile>,
    pub memory_notes: Vec<ContextMemoryNote>,
    pub recent_runs: Vec<RunMetadata>,
    pub compiled_context: String,
    pub estimated_tokens: u64,
    pub budget_report: BudgetEstimate,
}

pub struct ContextCompiler {
    root: PathBuf,
    agent_home: PathBuf,
    budget: TokenBudget,
}

impl ContextCompiler {
    pub fn new(
        root: impl Into<PathBuf>,
        agent_home: impl Into<PathBuf>,
        budget: TokenBudget,
    ) -> Self {
        Self {
            root: root.into(),
            agent_home: agent_home.into(),
            budget,
        }
    }

    pub fn compile(
        &self,
        task: &str,
        max_files: usize,
        max_memory_notes: usize,
    ) -> Result<ContextBundle, CoreError> {
        let repo_brain = RepoBrainBuilder::new(&self.root, &self.agent_home).build()?;
        let keywords = extract_keywords(task);
        let relevant_files = self.select_relevant_files(task, &keywords, max_files)?;
        let memory_notes = self.collect_memory_notes(max_memory_notes)?;
        let recent_runs = RunStore::new(self.agent_home.join("sessions"))
            .list_runs(5)
            .unwrap_or_default();

        let preliminary = render_context(
            task,
            &repo_brain,
            &relevant_files,
            &memory_notes,
            &recent_runs,
        );
        let preliminary_tokens = estimate_tokens(&preliminary);
        let budget_report = self.budget.estimate_prompt(preliminary_tokens);
        let recommended_level = budget_report.recommended_level;

        let relevant_files = if recommended_level == CompactLevel::Full {
            relevant_files
        } else {
            relevant_files
                .into_iter()
                .map(|mut file| {
                    let compact = compact_command_output(
                        &format!("read {}", file.path),
                        &file.excerpt,
                        recommended_level,
                    );
                    file.excerpt = compact.summary;
                    file.excerpt_tokens = compact.compact_tokens;
                    file
                })
                .collect()
        };

        let compiled_context = render_context(
            task,
            &repo_brain,
            &relevant_files,
            &memory_notes,
            &recent_runs,
        );
        let estimated_tokens = estimate_tokens(&compiled_context);
        let budget_report = self.budget.estimate_prompt(estimated_tokens);

        Ok(ContextBundle {
            task: task.to_string(),
            budget: self.budget.clone(),
            recommended_level,
            repo_brain,
            relevant_files,
            memory_notes,
            recent_runs,
            compiled_context,
            estimated_tokens,
            budget_report,
        })
    }

    fn select_relevant_files(
        &self,
        task: &str,
        keywords: &[String],
        max_files: usize,
    ) -> Result<Vec<ContextFile>, CoreError> {
        let mut scored = Vec::new();
        let files = collect_candidate_files(&self.root)?;
        for path in files {
            let relative = path
                .strip_prefix(&self.root)
                .unwrap_or(&path)
                .display()
                .to_string();
            let raw = match fs::read_to_string(&path) {
                Ok(raw) => raw,
                Err(_) => continue,
            };
            let score = file_score(&relative, &raw, keywords);
            if score <= 0 && !is_high_value_path(&relative) {
                continue;
            }
            let reason = file_reason(&relative, keywords);
            let excerpt = summarize_file(&raw, task);
            let raw_tokens = estimate_tokens(&raw);
            let excerpt_tokens = estimate_tokens(&excerpt);
            scored.push(ContextFile {
                path: relative,
                score,
                reason,
                excerpt,
                raw_tokens,
                excerpt_tokens,
            });
        }

        scored.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.path.len().cmp(&right.path.len()))
        });
        scored.truncate(max_files);
        Ok(scored)
    }

    fn collect_memory_notes(&self, max_notes: usize) -> Result<Vec<ContextMemoryNote>, CoreError> {
        let dir = self.agent_home.join("memory").join("project");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut scored = Vec::new();
        collect_memory_notes_recursive(&dir, &dir, &mut scored);
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_notes);
        Ok(scored.into_iter().map(|(_, note)| note).collect())
    }
}

fn collect_memory_notes_recursive(
    root: &std::path::Path,
    dir: &std::path::Path,
    scored: &mut Vec<(f64, ContextMemoryNote)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_memory_notes_recursive(root, &path, scored);
        } else if path.extension().and_then(|v| v.to_str()) == Some("md") {
            let key = if let Ok(rel) = path.strip_prefix(root) {
                let components: Vec<&str> = rel
                    .components()
                    .map(|c| c.as_os_str().to_str().unwrap_or(""))
                    .collect();
                if let Some((last, rest)) = components.split_last() {
                    let stem = last.trim_end_matches(".md");
                    let mut parts: Vec<&str> = rest.iter().copied().collect();
                    parts.push(stem);
                    parts.join(":")
                } else {
                    path.file_stem()
                        .and_then(|v| v.to_str())
                        .unwrap_or("note")
                        .to_string()
                }
            } else {
                path.file_stem()
                    .and_then(|v| v.to_str())
                    .unwrap_or("note")
                    .to_string()
            };

            let raw = match fs::read_to_string(&path) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let (content, meta) = parse_memory_frontmatter(&raw);
            let score = score_memory_entry(&meta);
            let excerpt = summarize_markdown_note(&content);
            scored.push((
                score,
                ContextMemoryNote {
                    key,
                    tokens: estimate_tokens(&excerpt),
                    excerpt,
                },
            ));
        }
    }
}

#[derive(Deserialize, Default)]
struct MemoryFrontmatterMeta {
    updated_at: Option<chrono::DateTime<Utc>>,
    access_count: Option<u32>,
}

fn parse_memory_frontmatter(raw: &str) -> (String, MemoryFrontmatterMeta) {
    if raw.starts_with("---\n") {
        if let Some(end) = raw[4..].find("\n---\n") {
            let yaml = &raw[4..4 + end];
            let content = raw[4 + end + 5..].to_string();
            let meta = serde_yaml::from_str::<MemoryFrontmatterMeta>(yaml).unwrap_or_default();
            return (content, meta);
        }
    }
    (raw.to_string(), MemoryFrontmatterMeta::default())
}

fn score_memory_entry(meta: &MemoryFrontmatterMeta) -> f64 {
    let count = meta.access_count.unwrap_or(0) as f64;
    let days_ago = meta
        .updated_at
        .map(|dt| (Utc::now() - dt).num_seconds() as f64 / 86400.0)
        .unwrap_or(365.0);
    0.4 * (count + 1.0).ln() + 0.6 * (-days_ago / 30.0).exp()
}

fn render_context(
    task: &str,
    repo_brain: &RepoBrain,
    relevant_files: &[ContextFile],
    memory_notes: &[ContextMemoryNote],
    recent_runs: &[RunMetadata],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("Task: {}\n\n", task));
    out.push_str(&format!("Repo brain: {}\n", repo_brain.summary));
    if !repo_brain.conventions.is_empty() {
        out.push_str("Conventions:\n");
        for item in &repo_brain.conventions {
            out.push_str(&format!("- {}\n", item));
        }
    }
    if !repo_brain.recommended_commands.is_empty() {
        out.push_str("Recommended commands:\n");
        for command in &repo_brain.recommended_commands {
            out.push_str(&format!("- {}\n", command));
        }
    }
    if !relevant_files.is_empty() {
        out.push_str("\nRelevant files:\n");
        for file in relevant_files {
            out.push_str(&format!(
                "* {} (score {}, reason: {})\n{}\n\n",
                file.path, file.score, file.reason, file.excerpt
            ));
        }
    }
    if !memory_notes.is_empty() {
        out.push_str("Project memory:\n");
        for note in memory_notes {
            out.push_str(&format!("* {}:\n{}\n", note.key, note.excerpt));
        }
    }
    if !recent_runs.is_empty() {
        out.push_str("Recent runs:\n");
        for run in recent_runs {
            out.push_str(&format!(
                "* {} [{}] {}\n",
                run.id,
                run.kind,
                run.output_preview.as_deref().unwrap_or("no preview"),
            ));
        }
    }
    out
}

fn collect_candidate_files(root: &Path) -> Result<Vec<PathBuf>, CoreError> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => return Err(CoreError::io(&dir, error)),
        };
        for entry in entries {
            let entry = entry.map_err(|error| CoreError::io(&dir, error))?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if is_ignored_dir(name) && path.is_dir() {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if is_candidate_file(&path) {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".next" | "dist" | "build" | ".idea" | ".vscode"
    )
}

fn is_candidate_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    matches!(
        ext,
        "rs" | "md" | "toml" | "yaml" | "yml" | "json" | "ts" | "tsx" | "js" | "jsx" | "py" | "go"
    ) || matches!(
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
        "Cargo.toml" | "package.json" | "AGENTS.md" | "README.md" | "README"
    )
}

fn extract_keywords(task: &str) -> Vec<String> {
    let stop = [
        "this", "that", "with", "from", "into", "about", "build", "make", "project", "feature",
        "agent007", "using", "want", "help", "work", "code", "for", "the", "and", "fix",
    ]
    .into_iter()
    .collect::<HashSet<_>>();

    let mut words = task
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .map(|word| word.trim().to_lowercase())
        .filter(|word| word.len() >= 3 && !stop.contains(word.as_str()))
        .collect::<Vec<_>>();
    words.sort();
    words.dedup();
    words
}

fn file_score(path: &str, raw: &str, keywords: &[String]) -> i32 {
    let path_lower = path.to_lowercase();
    let raw_lower = raw.to_lowercase();
    let mut score = if is_high_value_path(path) { 8 } else { 0 };
    for keyword in keywords {
        if path_lower.contains(keyword) {
            score += 6;
        }
        if raw_lower.contains(keyword) {
            score += 2;
        }
    }
    score
}

fn file_reason(path: &str, keywords: &[String]) -> String {
    let path_lower = path.to_lowercase();
    let mut hits = keywords
        .iter()
        .filter(|keyword| path_lower.contains(keyword.as_str()))
        .take(3)
        .cloned()
        .collect::<Vec<_>>();
    if is_high_value_path(path) {
        hits.insert(0, "high-value project file".to_string());
    }
    if hits.is_empty() {
        "fallback project context".to_string()
    } else {
        hits.join(", ")
    }
}

fn is_high_value_path(path: &str) -> bool {
    matches!(
        path,
        "AGENTS.md" | "README.md" | "Cargo.toml" | "package.json"
    ) || path.starts_with("src/")
        || path.starts_with("crates/")
}

fn summarize_file(raw: &str, task: &str) -> String {
    let command = if raw.lines().count() > 80 || raw.chars().count() > 2_000 {
        "read aggressive"
    } else {
        "read compact"
    };
    let compact = compact_command_output(command, raw, CompactLevel::Compact);
    let mut summary = compact.summary;
    if !task.trim().is_empty() {
        summary = format!("Task relevance: {}\n{}", task, summary);
    }
    summary
}

fn summarize_markdown_note(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .take(8)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_picks_relevant_files_and_memory() {
        let root = tempfile::tempdir().unwrap();
        let agent_home = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(root.path().join("AGENTS.md"), "Focus on auth and tests\n").unwrap();
        fs::create_dir_all(root.path().join("src")).unwrap();
        fs::write(root.path().join("src/auth.rs"), "pub fn auth_token() {}\n").unwrap();
        fs::write(root.path().join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        fs::create_dir_all(agent_home.path().join("memory").join("project")).unwrap();
        fs::write(
            agent_home
                .path()
                .join("memory")
                .join("project")
                .join("auth.md"),
            "# auth\nUse token-based auth\n",
        )
        .unwrap();

        let compiler = ContextCompiler::new(root.path(), agent_home.path(), TokenBudget::default());
        let bundle = compiler.compile("fix auth token bug", 4, 4).unwrap();
        assert!(!bundle.relevant_files.is_empty());
        assert!(bundle
            .relevant_files
            .iter()
            .any(|file| file.path.ends_with("src/auth.rs")));
        assert_eq!(bundle.memory_notes.len(), 1);
        assert!(bundle.compiled_context.contains("Repo brain"));
    }
}
