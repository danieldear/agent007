use std::path::PathBuf;
use std::sync::Arc;
use agent007_models::provider::ModelProvider;
use agent007_models::types::{CompletionRequest, Message, Role};

pub struct OptimizerConfig {
    pub threshold: f32,
    pub trigger_count: usize,
    pub optimizer_model: String,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            threshold: 0.3,
            trigger_count: 10,
            optimizer_model: "claude".to_string(),
        }
    }
}

pub struct PromptOptimizer {
    config: OptimizerConfig,
    provider: Arc<dyn ModelProvider>,
    learning_dispatcher: Arc<crate::dispatcher::LearningDispatcher>,
    /// Optional directory containing skill `.md` files. When set, successful
    /// optimizations write the improved prompt template back to disk so the
    /// next skill execution picks it up without a restart.
    skills_dir: Option<PathBuf>,
}

impl PromptOptimizer {
    pub fn new(
        config: OptimizerConfig,
        provider: Arc<dyn ModelProvider>,
        learning_dispatcher: Arc<crate::dispatcher::LearningDispatcher>,
    ) -> Self {
        Self {
            config,
            provider,
            learning_dispatcher,
            skills_dir: None,
        }
    }

    /// Attach a skill directory so the optimizer can persist improved prompts
    /// back to their source `.md` files on disk.
    pub fn with_skills_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.skills_dir = Some(dir.into());
        self
    }

    pub async fn maybe_optimize(
        &self,
        skill_name: &str,
        store: &crate::store::LearningStore,
        skill_prompt: &str,
    ) -> Result<(), crate::error::LearningError> {
        // Retrieve recent feedback entries
        let entries = store.list_recent_feedback(skill_name, self.config.trigger_count * 2)?;

        // Need at least trigger_count entries before running
        if entries.len() < self.config.trigger_count {
            return Ok(());
        }

        // Compute average reward
        let rewards: Vec<f32> = entries
            .iter()
            .filter_map(|e| e.reward)
            .collect();

        let avg_reward = if rewards.is_empty() {
            0.0f32
        } else {
            rewards.iter().sum::<f32>() / rewards.len() as f32
        };

        // Only optimize if below threshold
        if avg_reward >= self.config.threshold {
            return Ok(());
        }

        // TODO(P3-T10): The plan requires LanceDB RAG similarity search here to find
        // semantically similar failed entries (not just recent ones). Currently using
        // a simple linear scan of recent entries as a v0.1 placeholder. When the
        // LanceDB vector store is wired into LearningStore, replace this with a
        // similarity query filtered to Failure/ToolError outcomes.

        // Build failure examples string
        let failure_examples: String = entries
            .iter()
            .filter_map(|e| match &e.outcome {
                crate::types::Outcome::Failure { reason } => {
                    Some(format!("- Failure: {}", reason))
                }
                crate::types::Outcome::ToolError { tool } => {
                    Some(format!("- ToolError: {}", tool))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let system_instruction = format!(
            "You are a prompt engineer. The following skill prompt has been producing \
            poor results (average reward: {avg_reward:.2})."
        );

        let user_message = format!(
            "Here are examples of its failures:\n\n\
            {failure_examples}\n\n\
            Rewrite the prompt to fix these failure patterns. Keep the same goal.\n\
            Return only the improved prompt text.\n\n\
            Original prompt:\n{skill_prompt}"
        );

        // Call provider with meta-prompt
        let request = CompletionRequest {
            model: self.config.optimizer_model.clone(),
            messages: vec![Message {
                role: Role::User,
                content: user_message,
            }],
            max_tokens: None,
            temperature: None,
            system: Some(system_instruction),
        };

        let response = self.provider.complete(request).await?;
        let improved_prompt = response.content;

        // Determine next version number
        let existing_versions = store.get_prompt_versions(skill_name)?;
        let next_version = existing_versions
            .iter()
            .map(|v| v.version)
            .max()
            .unwrap_or(0)
            + 1;

        // Save new PromptVersion
        let version = crate::store::PromptVersion {
            version: next_version,
            skill_name: skill_name.to_string(),
            prompt_text: improved_prompt,
            avg_reward: Some(avg_reward),
            created_at: chrono::Utc::now(),
        };
        store.save_prompt_version(version.clone())?;

        // Write the improved prompt template back to the skill .md file on disk
        // so that the next execution picks it up without requiring a restart.
        if let Some(ref skills_dir) = self.skills_dir {
            self.persist_to_skill_file(skills_dir, skill_name, &version.prompt_text);
        }

        // Emit OptimizerTriggered event
        self.learning_dispatcher.publish(crate::types::LearningEvent::OptimizerTriggered {
            skill_name: skill_name.to_string(),
        })?;

        Ok(())
    }

    /// Rewrite the template body of a skill `.md` file in `skills_dir` while
    /// preserving the YAML frontmatter.  Errors are logged but not propagated —
    /// a failed write-back is non-fatal; the improved prompt is already persisted
    /// in the learning store.
    fn persist_to_skill_file(&self, skills_dir: &PathBuf, skill_name: &str, new_template: &str) {
        // Skill files are stored as `<trigger-or-name>.md`. Try both the plain
        // name and a slugified version (spaces → hyphens, lower-cased).
        let candidates = [
            skills_dir.join(format!("{}.md", skill_name)),
            skills_dir.join(format!("{}.md", skill_name.to_lowercase().replace(' ', "-"))),
        ];

        let skill_path = match candidates.iter().find(|p| p.exists()) {
            Some(p) => p.clone(),
            None => {
                tracing::warn!(
                    skill = skill_name,
                    "optimizer: skill file not found in {:?}; skipping write-back",
                    skills_dir
                );
                return;
            }
        };

        let content = match std::fs::read_to_string(&skill_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(skill = skill_name, error = %e, "optimizer: failed to read skill file for write-back");
                return;
            }
        };

        // Skill files use YAML frontmatter delimited by "---".
        // Split into at most 3 parts: ["", frontmatter, old_template].
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            tracing::warn!(skill = skill_name, "optimizer: skill file has unexpected format; skipping write-back");
            return;
        }

        let updated = format!("---{}---\n{}\n", parts[1], new_template.trim());
        if let Err(e) = std::fs::write(&skill_path, &updated) {
            tracing::warn!(skill = skill_name, error = %e, "optimizer: failed to write back improved prompt");
        } else {
            tracing::info!(skill = skill_name, path = ?skill_path, "optimizer: wrote improved prompt to skill file");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::types::{AgentId, PromptRef};
    use agent007_memory::store::MemoryStore;
    use agent007_models::mock::MockProvider;
    use crate::dispatcher::LearningDispatcher;
    use crate::store::LearningStore;
    use crate::types::{FeedbackEntry, LearningEvent, Outcome};
    use futures::StreamExt as _;
    use std::sync::Arc;
    use tempfile::TempDir;
    use uuid::Uuid;
    use chrono::Utc;

    fn make_store() -> (LearningStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let ms = Arc::new(MemoryStore::new(dir.path()));
        let scoped = ms.scoped("learning");
        (LearningStore::new(scoped), dir)
    }

    fn make_entry(skill: &str, outcome: Outcome, reward: f32) -> FeedbackEntry {
        FeedbackEntry {
            id: Uuid::new_v4(),
            agent_id: AgentId::new(),
            prompt_ref: PromptRef::new(),
            skill_name: Some(skill.to_string()),
            model: "claude".to_string(),
            outcome,
            reward: Some(reward),
            timestamp: Utc::now(),
        }
    }

    /// maybe_optimize() when avg_reward >= threshold does NOT call model or store new version
    #[tokio::test]
    async fn no_optimization_when_avg_reward_above_threshold() {
        let (store, _dir) = make_store();
        let dispatcher = Arc::new(LearningDispatcher::new(64));
        let provider = Arc::new(MockProvider::new("improved prompt text", "mock-model"));
        let config = OptimizerConfig {
            threshold: 0.3,
            trigger_count: 5,
            optimizer_model: "mock-model".to_string(),
        };
        let optimizer = PromptOptimizer::new(config, provider.clone(), dispatcher);

        // Insert 5 entries with high reward (0.5 > threshold 0.3)
        for _ in 0..5 {
            store.record_feedback(&make_entry("skill-a", Outcome::Success, 0.5)).unwrap();
        }

        optimizer.maybe_optimize("skill-a", &store, "original prompt").await.unwrap();

        // Provider should not be called
        assert_eq!(provider.call_count(), 0, "model should not be called when reward >= threshold");

        // No new version stored
        let versions = store.get_prompt_versions("skill-a").unwrap();
        assert!(versions.is_empty(), "no version should be stored");
    }

    /// maybe_optimize() when avg_reward < threshold AND entry count >= trigger_count:
    /// calls ModelProvider once, stores PromptVersion, emits OptimizerTriggered event
    #[tokio::test]
    async fn optimization_triggered_when_reward_below_threshold_and_enough_entries() {
        let (store, _dir) = make_store();
        let dispatcher = Arc::new(LearningDispatcher::new(64));
        let mut event_stream = dispatcher.subscribe();

        let provider = Arc::new(MockProvider::new("improved prompt text", "mock-model"));
        let config = OptimizerConfig {
            threshold: 0.3,
            trigger_count: 5,
            optimizer_model: "mock-model".to_string(),
        };
        let optimizer = PromptOptimizer::new(config, provider.clone(), dispatcher);

        // Insert 5 entries with low reward (0.2 < threshold 0.3)
        for _ in 0..5 {
            store.record_feedback(
                &make_entry("skill-b", Outcome::Failure { reason: "timeout".to_string() }, 0.2)
            ).unwrap();
        }

        optimizer.maybe_optimize("skill-b", &store, "original prompt").await.unwrap();

        // Provider should have been called once
        assert_eq!(provider.call_count(), 1, "model should be called once");

        // A new version should be stored
        let versions = store.get_prompt_versions("skill-b").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[0].prompt_text, "improved prompt text");
        assert_eq!(versions[0].skill_name, "skill-b");

        // OptimizerTriggered event should be emitted
        let event = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            event_stream.next(),
        )
        .await
        .expect("timed out waiting for event")
        .expect("stream ended");

        assert!(
            matches!(event, LearningEvent::OptimizerTriggered { ref skill_name } if skill_name == "skill-b"),
            "expected OptimizerTriggered for skill-b"
        );
    }

    /// maybe_optimize() when entry count < trigger_count does NOT trigger optimization
    #[tokio::test]
    async fn no_optimization_when_entry_count_below_trigger() {
        let (store, _dir) = make_store();
        let dispatcher = Arc::new(LearningDispatcher::new(64));
        let provider = Arc::new(MockProvider::new("improved prompt text", "mock-model"));
        let config = OptimizerConfig {
            threshold: 0.3,
            trigger_count: 10,
            optimizer_model: "mock-model".to_string(),
        };
        let optimizer = PromptOptimizer::new(config, provider.clone(), dispatcher);

        // Insert only 3 entries (< trigger_count of 10), with low reward
        for _ in 0..3 {
            store.record_feedback(
                &make_entry("skill-c", Outcome::Failure { reason: "error".to_string() }, 0.1)
            ).unwrap();
        }

        optimizer.maybe_optimize("skill-c", &store, "original prompt").await.unwrap();

        // Provider should not be called
        assert_eq!(provider.call_count(), 0, "model should not be called when entry count < trigger_count");

        // No versions stored
        let versions = store.get_prompt_versions("skill-c").unwrap();
        assert!(versions.is_empty());
    }

    /// Saved prompt version number increments correctly on successive optimizations
    #[tokio::test]
    async fn version_number_increments_on_successive_optimizations() {
        let (store, _dir) = make_store();
        let dispatcher = Arc::new(LearningDispatcher::new(64));
        let provider = Arc::new(MockProvider::new("improved prompt text", "mock-model"));
        let config = OptimizerConfig {
            threshold: 0.3,
            trigger_count: 5,
            optimizer_model: "mock-model".to_string(),
        };
        let optimizer = PromptOptimizer::new(config, provider.clone(), dispatcher);

        // Insert enough low-reward entries
        for _ in 0..10 {
            store.record_feedback(
                &make_entry("skill-d", Outcome::ToolError { tool: "bash".to_string() }, 0.1)
            ).unwrap();
        }

        // First optimization — should produce version 1
        optimizer.maybe_optimize("skill-d", &store, "original prompt").await.unwrap();
        let versions = store.get_prompt_versions("skill-d").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 1);

        // Second optimization — should produce version 2
        optimizer.maybe_optimize("skill-d", &store, "original prompt").await.unwrap();
        let versions = store.get_prompt_versions("skill-d").unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].version, 2);

        // Provider called twice total
        assert_eq!(provider.call_count(), 2);
    }

    /// When `with_skills_dir` is set and the skill .md file exists, the optimizer
    /// writes the improved prompt template back to disk while preserving frontmatter.
    #[tokio::test]
    async fn optimizer_writes_back_to_skill_file_when_skills_dir_set() {
        let (store, _dir) = make_store();
        let dispatcher = Arc::new(LearningDispatcher::new(64));
        let provider = Arc::new(MockProvider::new("rewritten prompt body", "mock-model"));

        // Create a temporary skills directory with a matching skill file
        let skills_dir = TempDir::new().unwrap();
        let skill_file = skills_dir.path().join("skill-e.md");
        std::fs::write(
            &skill_file,
            "---\nname: skill-e\ndescription: original\ntrigger: /skill-e\nmodel: claude\n---\noriginal template body\n",
        )
        .unwrap();

        let config = OptimizerConfig {
            threshold: 0.3,
            trigger_count: 5,
            optimizer_model: "mock-model".to_string(),
        };
        let optimizer = PromptOptimizer::new(config, provider.clone(), dispatcher)
            .with_skills_dir(skills_dir.path());

        for _ in 0..5 {
            store.record_feedback(
                &make_entry("skill-e", Outcome::Failure { reason: "bad output".to_string() }, 0.1),
            )
            .unwrap();
        }

        optimizer.maybe_optimize("skill-e", &store, "original template body").await.unwrap();

        // Read back the skill file and verify the template was updated
        let updated = std::fs::read_to_string(&skill_file).unwrap();
        assert!(
            updated.contains("rewritten prompt body"),
            "skill file should contain the improved prompt; got:\n{updated}"
        );
        // Frontmatter must be preserved
        assert!(
            updated.contains("name: skill-e"),
            "frontmatter should be preserved; got:\n{updated}"
        );
    }
}
