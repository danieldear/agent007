use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::budget::{estimate_tokens, BudgetEstimate, CompactLevel, TokenBudget};
use crate::compact::compact_command_output;
use crate::error::CoreError;
use crate::repo_brain::{RepoBrain, RepoBrainBuilder};
use crate::repo_filter;
use crate::repo_graph::{context_bundle_for_query, load_or_build_graph};
use crate::repo_readiness::{write_repo_intelligence_readiness, RepoIntelligenceOptions};
use crate::run_store::{RunMetadata, RunStore};

const MAX_STRUCTURAL_CONTEXT_CHARS: usize = 6_000;
const MAX_FILE_EXCERPT_CHARS: usize = 4_000;
const MAX_MEMORY_NOTE_EXCERPT_CHARS: usize = 2_000;
const SQLITE_MEMORY_VALUE_MAX_CHARS: i64 = 16_000;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextPromptManifest {
    pub sections: Vec<ContextPromptSection>,
    pub total_tokens: u64,
    pub total_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPromptSection {
    pub name: String,
    pub tokens: u64,
    pub chars: usize,
    pub item_count: usize,
    pub included: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBundle {
    pub task: String,
    pub budget: TokenBudget,
    pub recommended_level: CompactLevel,
    pub repo_brain: RepoBrain,
    pub structural_files: Vec<String>,
    pub structural_context: String,
    pub relevant_files: Vec<ContextFile>,
    pub memory_notes: Vec<ContextMemoryNote>,
    pub recent_runs: Vec<RunMetadata>,
    pub compiled_context: String,
    pub estimated_tokens: u64,
    pub budget_report: BudgetEstimate,
    pub prompt_manifest: ContextPromptManifest,
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
        let (structural_context, structural_files) = self.collect_structural_context(task);
        let relevant_files = self.select_relevant_files(task, &keywords, max_files)?;
        let memory_notes = self.collect_memory_notes(task, &keywords, max_memory_notes)?;
        let recent_runs = RunStore::new(self.agent_home.join("sessions"))
            .list_runs(5)
            .unwrap_or_default();

        let preliminary = render_context(
            task,
            &repo_brain,
            &structural_context,
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
            &structural_context,
            &relevant_files,
            &memory_notes,
            &recent_runs,
        );
        let estimated_tokens = estimate_tokens(&compiled_context);
        let budget_report = self.budget.estimate_prompt(estimated_tokens);
        let prompt_manifest = build_prompt_manifest(
            task,
            &repo_brain,
            &structural_context,
            &relevant_files,
            &memory_notes,
            &recent_runs,
        );

        Ok(ContextBundle {
            task: task.to_string(),
            budget: self.budget.clone(),
            recommended_level,
            repo_brain,
            structural_files,
            structural_context,
            relevant_files,
            memory_notes,
            recent_runs,
            compiled_context,
            estimated_tokens,
            budget_report,
            prompt_manifest,
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
            let excerpt = bound_text(&summarize_file(&raw, task), MAX_FILE_EXCERPT_CHARS);
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

    fn collect_memory_notes(
        &self,
        task: &str,
        keywords: &[String],
        max_notes: usize,
    ) -> Result<Vec<ContextMemoryNote>, CoreError> {
        let dir = self.agent_home.join("memory").join("project");
        let mut scored = Vec::new();
        if dir.exists() {
            collect_memory_notes_recursive(&dir, &dir, keywords, &mut scored);
        }
        collect_sqlite_memory_notes(
            &self.agent_home.join("memory").join("memory.db"),
            "project",
            task,
            keywords,
            &mut scored,
        )?;
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut seen = HashSet::new();
        scored.retain(|(_, note)| seen.insert(note.key.clone()));
        scored.truncate(max_notes);
        Ok(scored.into_iter().map(|(_, note)| note).collect())
    }

    fn collect_structural_context(&self, task: &str) -> (String, Vec<String>) {
        let _ = write_repo_intelligence_readiness(
            &self.root,
            None,
            &RepoIntelligenceOptions::default(),
        );
        let Ok(graph) = load_or_build_graph(&self.root, None) else {
            return (String::new(), Vec::new());
        };
        let bundle = context_bundle_for_query(&graph, task, 6, 2);
        (
            bound_text(&bundle.text, MAX_STRUCTURAL_CONTEXT_CHARS),
            bundle.files,
        )
    }
}

fn collect_memory_notes_recursive(
    root: &std::path::Path,
    dir: &std::path::Path,
    keywords: &[String],
    scored: &mut Vec<(f64, ContextMemoryNote)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_memory_notes_recursive(root, &path, keywords, scored);
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
            let score = score_memory_entry(&key, &content, keywords, &meta);
            if score <= 0.0 {
                continue;
            }
            let excerpt = bound_text(
                &summarize_markdown_note(&content),
                MAX_MEMORY_NOTE_EXCERPT_CHARS,
            );
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

fn collect_sqlite_memory_notes(
    db_path: &std::path::Path,
    namespace: &str,
    task: &str,
    keywords: &[String],
    scored: &mut Vec<(f64, ContextMemoryNote)>,
) -> Result<(), CoreError> {
    if !db_path.exists() {
        return Ok(());
    }
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|error| CoreError::io(db_path, std::io::Error::other(error)))?;
    let mut rows = Vec::new();
    let summary_result = conn
        .prepare(
            "SELECT key, substr(COALESCE(NULLIF(summary, ''), value), 1, ?2), updated_at, access_count \
             FROM memory WHERE namespace = ?1",
        )
        .and_then(|mut stmt| {
            let mapped = stmt.query_map(
                rusqlite::params![namespace, SQLITE_MEMORY_VALUE_MAX_CHARS],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )?;
            for row in mapped.flatten() {
                rows.push(row);
            }
            Ok::<_, rusqlite::Error>(())
        });
    if summary_result.is_err() {
        let mut stmt = match conn.prepare(
            "SELECT key, substr(value, 1, ?2), updated_at, access_count \
             FROM memory WHERE namespace = ?1",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Ok(()),
        };
        let mapped = stmt
            .query_map(
                rusqlite::params![namespace, SQLITE_MEMORY_VALUE_MAX_CHARS],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .map_err(|error| CoreError::io(db_path, std::io::Error::other(error)))?;
        for row in mapped.flatten() {
            rows.push(row);
        }
    }
    for (key, value, updated_at, access_count) in rows {
        let meta = MemoryFrontmatterMeta {
            updated_at: updated_at
                .as_deref()
                .and_then(|raw| raw.parse::<chrono::DateTime<Utc>>().ok()),
            access_count: access_count.map(|value| value.max(0) as u32),
        };
        let score = score_memory_entry(&key, &value, keywords, &meta);
        if score <= 0.0 && !task.trim().is_empty() {
            continue;
        }
        let excerpt = bound_text(
            &summarize_markdown_note(&value),
            MAX_MEMORY_NOTE_EXCERPT_CHARS,
        );
        scored.push((
            score,
            ContextMemoryNote {
                key,
                tokens: estimate_tokens(&excerpt),
                excerpt,
            },
        ));
    }
    Ok(())
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

fn score_memory_entry(
    key: &str,
    content: &str,
    keywords: &[String],
    meta: &MemoryFrontmatterMeta,
) -> f64 {
    let key_lower = key.to_lowercase();
    let content_lower = content.to_lowercase();
    let keyword_score = keywords
        .iter()
        .map(|keyword| {
            let mut score = 0.0;
            if key_lower.contains(keyword) {
                score += 2.0;
            }
            if content_lower.contains(keyword) {
                score += 1.0;
            }
            score
        })
        .sum::<f64>();
    let count = meta.access_count.unwrap_or(0) as f64;
    let days_ago = meta
        .updated_at
        .map(|dt| (Utc::now() - dt).num_seconds() as f64 / 86400.0)
        .unwrap_or(365.0);
    let base = 0.4 * (count + 1.0).ln() + 0.6 * (-days_ago / 30.0).exp();
    if keywords.is_empty() {
        base
    } else if keyword_score > 0.0 {
        keyword_score + base
    } else {
        0.0
    }
}

fn render_context(
    task: &str,
    repo_brain: &RepoBrain,
    structural_context: &str,
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
    if !structural_context.is_empty() {
        out.push_str("\nStructural repo graph:\n");
        out.push_str(structural_context);
        out.push('\n');
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

fn build_prompt_manifest(
    task: &str,
    repo_brain: &RepoBrain,
    structural_context: &str,
    relevant_files: &[ContextFile],
    memory_notes: &[ContextMemoryNote],
    recent_runs: &[RunMetadata],
) -> ContextPromptManifest {
    let mut sections = Vec::new();
    push_text_section(&mut sections, "task", task, 1, "user task");
    push_text_section(
        &mut sections,
        "repo_brain.summary",
        &repo_brain.summary,
        1,
        "compact project summary only",
    );
    push_text_section(
        &mut sections,
        "repo_brain.conventions",
        &repo_brain.conventions.join("\n"),
        repo_brain.conventions.len(),
        "top project instructions and conventions",
    );
    push_text_section(
        &mut sections,
        "repo_brain.recommended_commands",
        &repo_brain.recommended_commands.join("\n"),
        repo_brain.recommended_commands.len(),
        "validation commands",
    );
    push_text_section(
        &mut sections,
        "structural_repo_graph",
        structural_context,
        if structural_context.is_empty() { 0 } else { 1 },
        "bounded structural context from repo graph",
    );
    let relevant_text = relevant_files
        .iter()
        .map(|file| format!("{} {}\n{}", file.path, file.reason, file.excerpt))
        .collect::<Vec<_>>()
        .join("\n");
    push_text_section(
        &mut sections,
        "relevant_files",
        &relevant_text,
        relevant_files.len(),
        "bounded excerpts from selected relevant files",
    );
    let memory_text = memory_notes
        .iter()
        .map(|note| format!("{}:\n{}", note.key, note.excerpt))
        .collect::<Vec<_>>()
        .join("\n");
    push_text_section(
        &mut sections,
        "project_memory_notes",
        &memory_text,
        memory_notes.len(),
        "bounded memory snippets selected by task keywords",
    );
    let run_text = recent_runs
        .iter()
        .map(|run| {
            format!(
                "{} [{}] {}",
                run.id,
                run.kind,
                run.output_preview.as_deref().unwrap_or("no preview")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    push_text_section(
        &mut sections,
        "recent_runs",
        &run_text,
        recent_runs.len(),
        "short run previews only",
    );
    sections.push(ContextPromptSection {
        name: "repo_brain.memory_note_keys".to_string(),
        tokens: 0,
        chars: 0,
        item_count: repo_brain.memory_notes.len(),
        included: false,
        reason: "memory key inventory stays in metadata and is not rendered into the prompt"
            .to_string(),
    });
    let total_tokens = sections
        .iter()
        .filter(|section| section.included)
        .map(|section| section.tokens)
        .sum();
    let total_chars = sections
        .iter()
        .filter(|section| section.included)
        .map(|section| section.chars)
        .sum();
    ContextPromptManifest {
        sections,
        total_tokens,
        total_chars,
    }
}

fn push_text_section(
    sections: &mut Vec<ContextPromptSection>,
    name: &str,
    text: &str,
    item_count: usize,
    reason: &str,
) {
    sections.push(ContextPromptSection {
        name: name.to_string(),
        tokens: estimate_tokens(text),
        chars: text.chars().count(),
        item_count,
        included: !text.trim().is_empty(),
        reason: reason.to_string(),
    });
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
            if path.is_dir() && repo_filter::should_skip_dir_name(name) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if repo_filter::should_skip_prompt_path(&path)
                || !repo_filter::file_is_within_prompt_budget(&path)
            {
                continue;
            }
            if is_candidate_file(&path) {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn is_candidate_file(path: &Path) -> bool {
    if repo_filter::should_skip_prompt_path(path) {
        return false;
    }
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
        "AGENTS.md" | "README.md" | "Cargo.toml" | "package.json" | "src/lib.rs" | "src/main.rs"
    ) || path.ends_with("/Cargo.toml")
        || path.ends_with("/package.json")
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

fn bound_text(raw: &str, max_chars: usize) -> String {
    if raw.chars().count() <= max_chars {
        return raw.to_string();
    }
    let mut out = raw.chars().take(max_chars).collect::<String>();
    out.push_str("\n...[truncated by agent007 prompt hygiene budget]");
    out
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
        assert!(bundle.structural_context.contains("auth_token"));
        assert!(bundle
            .relevant_files
            .iter()
            .any(|file| file.path.ends_with("src/auth.rs")));
        assert_eq!(bundle.memory_notes.len(), 1);
        assert!(bundle.compiled_context.contains("Repo brain"));
        assert!(bundle.compiled_context.contains("Structural repo graph"));
        assert!(bundle.prompt_manifest.total_tokens > 0);
        assert!(bundle
            .prompt_manifest
            .sections
            .iter()
            .any(|section| section.name == "relevant_files" && section.included));
        assert!(bundle
            .prompt_manifest
            .sections
            .iter()
            .any(|section| { section.name == "repo_brain.memory_note_keys" && !section.included }));
    }

    #[test]
    fn compiler_excludes_agent007_runtime_and_session_files() {
        let root = tempfile::tempdir().unwrap();
        let agent_home = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join("src")).unwrap();
        fs::write(
            root.path().join("src/lib.rs"),
            "pub fn workflow_gate() {}\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join(".agent007").join("sessions")).unwrap();
        fs::write(
            root.path()
                .join(".agent007")
                .join("sessions")
                .join("context-bundle.json"),
            r#"{"task":"workflow gate approval","status":"running"}"#,
        )
        .unwrap();
        fs::create_dir_all(root.path().join(".agent007").join("runtime")).unwrap();
        fs::write(
            root.path()
                .join(".agent007")
                .join("runtime")
                .join("huge.json"),
            r#"{"workflow":"gate","approval":"runtime"}"#,
        )
        .unwrap();
        fs::create_dir_all(
            root.path()
                .join(".agent007.bak.20260503-142745")
                .join("sessions"),
        )
        .unwrap();
        fs::write(
            root.path()
                .join(".agent007.bak.20260503-142745")
                .join("sessions")
                .join("workflow-state.json"),
            r#"{"workflow":"gate","approval":"backup"}"#,
        )
        .unwrap();
        fs::write(
            root.path().join("package-lock.json"),
            r#"{"packages":{"node_modules/noise":{"version":"1.0.0"}}}"#,
        )
        .unwrap();
        fs::write(root.path().join(".env"), "OPENAI_API_KEY=secret").unwrap();

        let compiler = ContextCompiler::new(root.path(), agent_home.path(), TokenBudget::default());
        let bundle = compiler
            .compile("debug workflow gate approval", 8, 4)
            .unwrap();

        assert!(bundle
            .relevant_files
            .iter()
            .all(|file| !file.path.starts_with(".agent007/")));
        assert!(bundle
            .relevant_files
            .iter()
            .all(|file| !file.path.starts_with(".agent007.bak")));
        assert!(bundle
            .relevant_files
            .iter()
            .all(|file| file.path != "package-lock.json" && file.path != ".env"));
        assert!(bundle
            .relevant_files
            .iter()
            .any(|file| file.path == "src/lib.rs"));
    }

    #[test]
    fn compiler_does_not_promote_unrelated_crate_files_without_relevance() {
        let root = tempfile::tempdir().unwrap();
        let agent_home = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/core\", \"crates/web\"]\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join("crates/core/src")).unwrap();
        fs::create_dir_all(root.path().join("crates/web/src")).unwrap();
        fs::write(
            root.path().join("crates/core/src/context.rs"),
            "pub fn prompt_manifest_token_accounting() {}\n",
        )
        .unwrap();
        fs::write(
            root.path().join("crates/web/src/api.rs"),
            "pub fn websocket_dashboard_routes() {}\n",
        )
        .unwrap();

        let compiler = ContextCompiler::new(root.path(), agent_home.path(), TokenBudget::default());
        let bundle = compiler
            .compile("fix prompt manifest token accounting", 8, 4)
            .unwrap();

        assert!(bundle
            .relevant_files
            .iter()
            .any(|file| file.path == "crates/core/src/context.rs"));
        assert!(bundle
            .relevant_files
            .iter()
            .all(|file| file.path != "crates/web/src/api.rs"));
    }

    #[test]
    fn compiler_excludes_generated_agents_companion_when_agents_md_exists() {
        let root = tempfile::tempdir().unwrap();
        let agent_home = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(
            root.path().join("AGENTS.md"),
            "Use agent007 for login and JWT security audits\n",
        )
        .unwrap();
        fs::write(
            root.path().join("AGENTS.agent007.generated.md"),
            "Use agent007 for login and JWT security audits\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join("src")).unwrap();
        fs::write(
            root.path().join("src/auth.rs"),
            "pub fn login_with_jwt() {}\n",
        )
        .unwrap();

        let compiler = ContextCompiler::new(root.path(), agent_home.path(), TokenBudget::default());
        let bundle = compiler
            .compile("security audit login JWT handling", 8, 4)
            .unwrap();

        assert!(bundle
            .relevant_files
            .iter()
            .any(|file| file.path == "AGENTS.md"));
        assert!(bundle
            .relevant_files
            .iter()
            .all(|file| file.path != "AGENTS.agent007.generated.md"));
    }
}
