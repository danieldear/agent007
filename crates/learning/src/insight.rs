use std::collections::HashMap;
use std::sync::Arc;

use agent007_memory::store::{MemoryEntryType, MemoryMeta, ScopedMemoryStore};
use agent007_models::provider::ModelProvider;
use agent007_models::types::{CompletionRequest, Message, Role};
use chrono::Utc;

use crate::types::Outcome;

/// Configuration for automatic insight generation.
pub struct InsightConfig {
    /// Minimum number of feedback entries required before insight generation is triggered.
    pub min_feedback_count: usize,
    /// Minimum failure rate (0.0–1.0) required to generate an insight.
    /// An insight is only generated when failures are frequent enough to form a pattern.
    pub min_failure_rate: f32,
    /// Trigger a check every N new feedback entries for a skill.
    /// E.g. with check_every_n=5, insights are considered at 5, 10, 15, … entries.
    pub check_every_n: usize,
    /// Model identifier used to generate the insight text.
    pub insight_model: String,
    /// Maximum insights stored per skill. Once reached, new insights are suppressed
    /// until existing ones are pruned. Prevents unbounded memory growth.
    pub max_insights_per_skill: usize,
}

impl Default for InsightConfig {
    fn default() -> Self {
        Self {
            min_feedback_count: 5,
            min_failure_rate: 0.2,
            check_every_n: 5,
            insight_model: "claude".to_string(),
            max_insights_per_skill: 10,
        }
    }
}

/// A procedural memory entry auto-generated from skill usage patterns.
#[derive(Debug, Clone)]
pub struct InsightEntry {
    pub skill_name: String,
    /// The memory key under which this insight was written in the project scope.
    pub memory_key: String,
    /// The generated insight text (Markdown, "## Insight: …" heading + body).
    pub content: String,
    /// Failure rate observed at the time of generation (0.0–1.0).
    pub failure_rate: f32,
    /// Number of feedback entries sampled.
    pub sample_size: usize,
}

/// Watches feedback patterns for individual skills and automatically writes
/// `type: procedural` memory entries when recurring failure patterns are detected.
///
/// ## How it works
///
/// After each feedback entry is recorded by `FeedbackCollector`, the collector
/// calls `InsightGenerator::maybe_generate(skill_name, store)`. The generator:
///
/// 1. Loads the N most recent feedback entries for the skill.
/// 2. Computes the failure rate and groups common failure reasons.
/// 3. If the failure rate exceeds `min_failure_rate`, calls the configured LLM
///    to produce a concise procedural rule (2–3 sentences).
/// 4. Writes the insight to the project memory scope as `type: procedural` so
///    `{{memory.project}}` and `{{rag_context}}` can surface it in future prompts.
/// 5. Maintains a per-skill index to avoid exceeding `max_insights_per_skill`.
pub struct InsightGenerator {
    pub config: InsightConfig,
    provider: Arc<dyn ModelProvider>,
    /// Scoped to "project" — insights land alongside hand-written memory entries.
    project_scope: ScopedMemoryStore,
}

impl InsightGenerator {
    pub fn new(
        config: InsightConfig,
        provider: Arc<dyn ModelProvider>,
        project_scope: ScopedMemoryStore,
    ) -> Self {
        Self {
            config,
            provider,
            project_scope,
        }
    }

    /// Evaluate whether an insight should be generated for `skill_name`.
    ///
    /// Returns `Ok(Some(InsightEntry))` if an insight was written to memory,
    /// `Ok(None)` if conditions were not met (not enough data, low failure rate,
    /// or the per-skill cap has been reached).
    pub async fn maybe_generate(
        &self,
        skill_name: &str,
        store: &crate::store::LearningStore,
    ) -> Result<Option<InsightEntry>, crate::error::LearningError> {
        // ── 1. Load feedback ────────────────────────────────────────────────────
        let window = self.config.min_feedback_count * 4;
        let entries = store.list_recent_feedback(skill_name, window)?;
        if entries.len() < self.config.min_feedback_count {
            return Ok(None);
        }

        // ── 2. Compute failure stats ─────────────────────────────────────────────
        let total = entries.len();
        let failure_reasons: Vec<String> = entries
            .iter()
            .filter_map(|e| match &e.outcome {
                Outcome::Failure { reason } => Some(reason.clone()),
                Outcome::ToolError { tool } => Some(format!("tool error: {tool}")),
                _ => None,
            })
            .collect();
        let failure_rate = failure_reasons.len() as f32 / total as f32;

        if failure_rate < self.config.min_failure_rate {
            return Ok(None);
        }

        // ── 3. Check per-skill cap ───────────────────────────────────────────────
        let index_key = format!("insight_index/{}", skill_name);
        let existing: Vec<String> = self
            .project_scope
            .read(&index_key)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if existing.len() >= self.config.max_insights_per_skill {
            tracing::debug!(
                skill = skill_name,
                cap = self.config.max_insights_per_skill,
                "insight cap reached — skipping generation"
            );
            return Ok(None);
        }

        // ── 4. Aggregate top failure reasons ────────────────────────────────────
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for r in &failure_reasons {
            *counts.entry(r.as_str()).or_insert(0) += 1;
        }
        let mut sorted: Vec<(&&str, &usize)> = counts.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        let top_reasons: Vec<String> = sorted
            .iter()
            .take(3)
            .map(|(r, c)| format!("{} ({c}×)", r))
            .collect();

        let success_pct = ((1.0 - failure_rate) * 100.0) as u32;
        let failure_pct = (failure_rate * 100.0) as u32;

        // ── 5. Call LLM ─────────────────────────────────────────────────────────
        let prompt = format!(
            "You are analyzing usage patterns for the agent007 skill \"{skill_name}\".\n\
             \n\
             Recent feedback summary:\n\
             - Total executions: {total}\n\
             - Success rate: {success_pct}%\n\
             - Failure rate: {failure_pct}%\n\
             - Common failure reasons: {reasons}\n\
             \n\
             Generate a single concise procedural insight (2–3 sentences) that would help \
             future users of this skill avoid these failures. Focus on actionable advice.\n\
             \n\
             Format your response exactly as:\n\
             ## Insight: <short title>\n\
             <concise advice>",
            skill_name = skill_name,
            total = total,
            success_pct = success_pct,
            failure_pct = failure_pct,
            reasons = top_reasons.join("; "),
        );

        let req = CompletionRequest {
            model: self.config.insight_model.clone(),
            messages: vec![Message {
                role: Role::User,
                content: prompt,
            }],
            max_tokens: None,
            temperature: None,
            system: None,
        };
        let response = self.provider.complete(req).await.map_err(|e| {
            crate::error::LearningError::OptimizerFailed {
                skill: skill_name.to_string(),
                reason: format!("insight LLM call failed: {e}"),
            }
        })?;
        let content = response.content.trim().to_string();

        // ── 6. Write procedural memory entry ────────────────────────────────────
        let ts = Utc::now().format("%Y%m%d%H%M%S");
        let key = format!("insight_{}_{}", skill_name, ts);

        // Extract the heading as a short summary for the memory frontmatter.
        let summary = content
            .lines()
            .next()
            .unwrap_or("Auto-generated insight")
            .trim_start_matches("## Insight: ")
            .to_string();

        let meta = MemoryMeta {
            entry_type: MemoryEntryType::Procedural,
            summary,
            confidence: 0.8,
            related_to: vec![skill_name.to_string()],
            ..MemoryMeta::default()
        };
        self.project_scope.write_with_meta(&key, &content, meta)?;

        // ── 7. Update per-skill index ────────────────────────────────────────────
        let mut keys = existing;
        keys.push(key.clone());
        let index_json = serde_json::to_string(&keys)?;
        self.project_scope.write(&index_key, &index_json)?;

        tracing::info!(
            skill = skill_name,
            key = %key,
            failure_rate = failure_rate,
            sample_size = total,
            "auto-insight generated"
        );

        Ok(Some(InsightEntry {
            skill_name: skill_name.to_string(),
            memory_key: key,
            content,
            failure_rate,
            sample_size: total,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::LearningStore;
    use crate::types::{FeedbackEntry, Outcome};
    use agent007_core::types::{AgentId, PromptRef};
    use agent007_memory::store::MemoryStore;
    use agent007_models::provider::ModelProvider;
    use agent007_models::types::{CompletionRequest, CompletionResponse};
    use async_trait::async_trait;
    use std::sync::Arc;
    use tempfile::TempDir;
    use uuid::Uuid;

    struct StubProvider {
        response: String,
        should_fail: bool,
    }

    impl StubProvider {
        fn ok(response: impl Into<String>) -> Arc<Self> {
            Arc::new(Self {
                response: response.into(),
                should_fail: false,
            })
        }
        fn failing() -> Arc<Self> {
            Arc::new(Self {
                response: String::new(),
                should_fail: true,
            })
        }
    }

    #[async_trait]
    impl ModelProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, agent007_models::ModelError> {
            if self.should_fail {
                Err(agent007_models::ModelError::NotConfigured(
                    "stub".to_string(),
                ))
            } else {
                Ok(CompletionResponse {
                    content: self.response.clone(),
                    model: "stub".to_string(),
                    input_tokens: None,
                    output_tokens: None,
                    cached_tokens: None,
                    cache_write_tokens: None,
                    total_tokens: None,
                    estimated_cost_usd: None,
                })
            }
        }
    }

    fn make_setup(dir: &TempDir) -> (LearningStore, Arc<MemoryStore>) {
        let ms = Arc::new(MemoryStore::new(dir.path()));
        let learning_scope = ms.scoped("learning");
        (LearningStore::new(learning_scope), ms)
    }

    fn make_entry(skill: &str, outcome: Outcome) -> FeedbackEntry {
        FeedbackEntry {
            id: Uuid::new_v4(),
            agent_id: AgentId::new(),
            prompt_ref: PromptRef::new(),
            skill_name: Some(skill.to_string()),
            model: "claude".to_string(),
            outcome,
            reward: Some(0.5),
            timestamp: chrono::Utc::now(),
        }
    }

    /// Returns None when there are fewer entries than min_feedback_count.
    #[tokio::test]
    async fn no_insight_below_min_feedback_count() {
        let dir = TempDir::new().unwrap();
        let (store, ms) = make_setup(&dir);

        // Record only 2 entries (below default min of 5)
        for _ in 0..2 {
            store
                .record_feedback(&make_entry(
                    "test-skill",
                    Outcome::Failure {
                        reason: "timeout".to_string(),
                    },
                ))
                .unwrap();
        }

        let gen = InsightGenerator::new(
            InsightConfig::default(),
            StubProvider::ok("## Insight: Test\nsome advice"),
            ms.scoped("project"),
        );
        let result = gen.maybe_generate("test-skill", &store).await.unwrap();
        assert!(
            result.is_none(),
            "should not generate insight below threshold"
        );
    }

    /// Returns None when failure rate is below min_failure_rate.
    #[tokio::test]
    async fn no_insight_when_failure_rate_low() {
        let dir = TempDir::new().unwrap();
        let (store, ms) = make_setup(&dir);

        // 5 successes, 0 failures → failure rate = 0%
        for _ in 0..5 {
            store
                .record_feedback(&make_entry("good-skill", Outcome::Success))
                .unwrap();
        }

        let gen = InsightGenerator::new(
            InsightConfig {
                min_failure_rate: 0.2,
                ..InsightConfig::default()
            },
            StubProvider::ok("## Insight: X\nadvice"),
            ms.scoped("project"),
        );
        let result = gen.maybe_generate("good-skill", &store).await.unwrap();
        assert!(
            result.is_none(),
            "should not generate insight when failures are rare"
        );
    }

    /// Generates an insight and writes it to project scope when failure rate exceeds threshold.
    #[tokio::test]
    async fn generates_insight_when_failure_rate_high() {
        let dir = TempDir::new().unwrap();
        let (store, ms) = make_setup(&dir);

        // 2 successes, 3 failures → failure rate = 60%
        store
            .record_feedback(&make_entry("rtt-analyze", Outcome::Success))
            .unwrap();
        store
            .record_feedback(&make_entry("rtt-analyze", Outcome::Success))
            .unwrap();
        for _ in 0..3 {
            store
                .record_feedback(&make_entry(
                    "rtt-analyze",
                    Outcome::Failure {
                        reason: "negative bias".to_string(),
                    },
                ))
                .unwrap();
        }

        let insight_text = "## Insight: Check AP antenna height for negative bias\nWhen RTT shows negative bias, check AP antenna height before adjusting algorithm parameters.";
        let gen = InsightGenerator::new(
            InsightConfig::default(),
            StubProvider::ok(insight_text),
            ms.scoped("project"),
        );

        let result = gen.maybe_generate("rtt-analyze", &store).await.unwrap();
        assert!(result.is_some(), "expected insight to be generated");

        let entry = result.unwrap();
        assert_eq!(entry.skill_name, "rtt-analyze");
        assert!(entry.failure_rate > 0.5);
        assert_eq!(entry.sample_size, 5);
        assert!(entry.memory_key.starts_with("insight_rtt-analyze_"));

        // Verify it was written to project memory (second handle, same underlying store)
        let verify_scope = ms.scoped("project");
        let stored = verify_scope.read(&entry.memory_key).unwrap();
        assert!(
            stored.is_some(),
            "insight should be persisted in project scope"
        );
        assert!(stored.unwrap().contains("negative bias"));
    }

    /// Suppresses generation when per-skill cap is reached.
    #[tokio::test]
    async fn suppresses_insight_at_cap() {
        let dir = TempDir::new().unwrap();
        let (store, ms) = make_setup(&dir);
        let project_scope = ms.scoped("project");

        // Pre-populate the index with max_insights_per_skill entries
        let existing: Vec<String> = (0..2).map(|i| format!("insight_capped_{i}")).collect();
        let index_json = serde_json::to_string(&existing).unwrap();
        project_scope
            .write("insight_index/capped-skill", &index_json)
            .unwrap();

        // 0 successes, 5 failures → high failure rate
        for _ in 0..5 {
            store
                .record_feedback(&make_entry(
                    "capped-skill",
                    Outcome::Failure {
                        reason: "err".to_string(),
                    },
                ))
                .unwrap();
        }

        let gen = InsightGenerator::new(
            InsightConfig {
                max_insights_per_skill: 2,
                ..InsightConfig::default()
            },
            StubProvider::ok("## Insight: X\nadvice"),
            project_scope,
        );
        let result = gen.maybe_generate("capped-skill", &store).await.unwrap();
        assert!(result.is_none(), "should suppress when cap is reached");
    }

    /// When the LLM call fails, returns LearningError (not a panic).
    #[tokio::test]
    async fn model_failure_returns_error() {
        let dir = TempDir::new().unwrap();
        let (store, ms) = make_setup(&dir);

        for _ in 0..5 {
            store
                .record_feedback(&make_entry(
                    "fail-skill",
                    Outcome::Failure {
                        reason: "x".to_string(),
                    },
                ))
                .unwrap();
        }

        let gen = InsightGenerator::new(
            InsightConfig::default(),
            StubProvider::failing(),
            ms.scoped("project"),
        );
        let result = gen.maybe_generate("fail-skill", &store).await;
        assert!(
            result.is_err(),
            "expected LearningError when model is unavailable"
        );
    }

    /// Index is updated after a successful insight write.
    #[tokio::test]
    async fn index_updated_after_insight_write() {
        let dir = TempDir::new().unwrap();
        let (store, ms) = make_setup(&dir);

        for _ in 0..5 {
            store
                .record_feedback(&make_entry(
                    "indexed-skill",
                    Outcome::Failure {
                        reason: "nlos".to_string(),
                    },
                ))
                .unwrap();
        }

        let gen = InsightGenerator::new(
            InsightConfig::default(),
            StubProvider::ok("## Insight: NLOS issue\nCheck for NLOS before analysis."),
            ms.scoped("project"),
        );
        gen.maybe_generate("indexed-skill", &store).await.unwrap();

        let verify_scope = ms.scoped("project");
        let index_raw = verify_scope.read("insight_index/indexed-skill").unwrap();
        assert!(index_raw.is_some(), "insight index should exist");
        let keys: Vec<String> = serde_json::from_str(&index_raw.unwrap()).unwrap();
        assert_eq!(keys.len(), 1, "one insight should be indexed");
        assert!(keys[0].starts_with("insight_indexed-skill_"));
    }
}
