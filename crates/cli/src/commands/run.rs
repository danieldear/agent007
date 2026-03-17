use std::sync::Arc;
use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::config::Config;
use agent007_core::dispatcher::LocalDispatcher;
use agent007_memory::store::MemoryStore;
use agent007_hooks::{HookConfig, HookExecutor};
use agent007_mcp::{McpClient, McpServerConfig};
use agent007_learning::{FeedbackCollector, LearningDispatcher, RewardScorer};
use agent007_learning::scorer::RewardWeights;
use agent007_learning::store::LearningStore;
use agent007_tui::{App, EventLoop};

pub struct Stack {
    pub dispatcher: Arc<LocalDispatcher>,
    pub memory_store: Arc<MemoryStore>,
    pub hook_executor: Arc<HookExecutor>,
    pub mcp_client: Arc<McpClient>,
    pub feedback_collector: Arc<FeedbackCollector>,
    pub learning_dispatcher: Arc<LearningDispatcher>,
    pub cancel: CancellationToken,
    pub tracker: TaskTracker,
}

pub async fn build_stack(config: &Config) -> Result<Stack> {
    let cancel = CancellationToken::new();
    let tracker = TaskTracker::new();

    // 1. Core dispatcher — returns Arc<LocalDispatcher>
    let dispatcher = LocalDispatcher::new(config.core.task_queue_capacity);

    // 2. Memory store
    let memory_dir = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".agent007")
        .join("memory");
    let memory_store = Arc::new(MemoryStore::new(memory_dir));

    // 3. Hook executor — load from file or use defaults
    let hooks_path = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".agent007")
        .join("hooks")
        .join("hooks.toml");
    let hook_config = HookConfig::load(&hooks_path).unwrap_or_default();
    let hook_executor = Arc::new(HookExecutor::new(hook_config));

    // 4. MCP client (from config)
    let mcp_servers: Vec<McpServerConfig> = config
        .mcp
        .as_ref()
        .map(|m| {
            m.servers
                .iter()
                .map(|(name, cmd)| McpServerConfig {
                    name: name.clone(),
                    command: cmd.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mcp_client = Arc::new(McpClient::new(mcp_servers));

    // 5. Learning dispatcher — new() returns Self, wrap in Arc
    let learning_dispatcher = Arc::new(LearningDispatcher::new(512));

    // 6. Learning store — scoped() requires &Arc<MemoryStore>
    let learning_store = LearningStore::new(memory_store.scoped("learning"));

    // 7. Reward scorer
    let reward_weights = RewardWeights {
        completion: config.learning.reward_weights.completion,
        user_rating: config.learning.reward_weights.user_rating,
        tool_errors: config.learning.reward_weights.tool_errors,
        retries: config.learning.reward_weights.retries,
    };
    let scorer = RewardScorer::new(reward_weights);

    // 8. Feedback collector
    let feedback_collector = Arc::new(FeedbackCollector::new(
        dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        learning_store,
        scorer,
        learning_dispatcher.clone(),
    ));

    Ok(Stack {
        dispatcher,
        memory_store,
        hook_executor,
        mcp_client,
        feedback_collector,
        learning_dispatcher,
        cancel,
        tracker,
    })
}

pub async fn execute(config: Arc<Config>, task: String) -> Result<()> {
    let stack = build_stack(&config).await?;

    // Spawn feedback collector
    let collector = stack.feedback_collector.clone();
    stack.tracker.spawn(async move {
        if let Err(e) = collector.run().await {
            tracing::warn!("feedback collector error: {}", e);
        }
    });

    // When AGENT007_DRY_RUN=1, skip the TUI and just return Ok
    if std::env::var("AGENT007_DRY_RUN").is_ok() {
        stack.cancel.cancel();
        stack.tracker.close();
        // Do not await tracker.wait() in dry-run: the feedback-collector task holds a
        // broadcast stream that never ends, so waiting would block forever. The process
        // (or test) will clean up background tasks on drop.
        return Ok(());
    }

    // Construct App and EventLoop
    let mut app = App::default();
    app.push_log(format!("Starting task: {}", task));

    let event_loop = EventLoop::new(
        stack.dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        stack.learning_dispatcher.clone(),
    )
    .await?;

    event_loop.run(&mut app, stack.cancel.clone()).await?;
    stack.tracker.close();
    stack.tracker.wait().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[tokio::test]
    async fn run_command_builds_stack_without_panic() {
        let config = Config::default();
        let stack = build_stack(&config).await.unwrap();
        // Verify stack was constructed (just check fields exist)
        assert!(stack.cancel.is_cancelled() == false);
    }

    #[tokio::test]
    async fn e2e_smoke_run_with_dry_run() {
        std::env::set_var("AGENT007_DRY_RUN", "1");
        let config = Arc::new(Config::default());
        let result = execute(config, "say hello".to_string()).await;
        std::env::remove_var("AGENT007_DRY_RUN");
        assert!(result.is_ok(), "run command failed: {:?}", result.err());
    }
}
