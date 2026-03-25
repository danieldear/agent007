use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use agent007_core::dispatcher::Dispatcher;
use agent007_core::persona::PersonaProvider;
use agent007_models::{ModelRouter, ModelProvider, CompletionRequest, Message, Role};

use crate::approval::ApprovalGate;
use crate::dag::DagValidator;
use crate::error::WorkflowError;
use crate::types::{BudgetUsed, WorkflowDef, WorkflowResult};

pub struct WorkflowRunner {
    pub persona_provider: Arc<dyn PersonaProvider>,
    pub model_router: Arc<ModelRouter>,
    pub dispatcher: Arc<dyn Dispatcher>,
}

impl WorkflowRunner {
    pub fn new(
        persona_provider: Arc<dyn PersonaProvider>,
        model_router: Arc<ModelRouter>,
        dispatcher: Arc<dyn Dispatcher>,
    ) -> Self {
        Self { persona_provider, model_router, dispatcher }
    }

    /// Validate the DAG and return topological batches. Public so the CLI `validate` command
    /// can call it without running steps.
    pub fn validate(&self, def: &WorkflowDef) -> Result<Vec<Vec<String>>, WorkflowError> {
        DagValidator::new(def).validate()
    }

    /// Run the full workflow. `task_input` fills the `{{task}}` Tera variable.
    pub async fn run(
        &self,
        def: &WorkflowDef,
        task_input: &str,
    ) -> Result<WorkflowResult, WorkflowError> {
        let batches = self.validate(def)?;
        let steps_total = def.steps.len();

        // Shared output artifact store, protected by a Mutex for concurrent batch steps.
        let outputs: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let budget_used: Arc<Mutex<BudgetUsed>> = Arc::new(Mutex::new(BudgetUsed::default()));
        let mut steps_completed = 0_usize;

        // Build a lookup: step_id → StepDef
        let step_map: HashMap<String, _> = def.steps.iter()
            .map(|s| (s.id.clone(), s))
            .collect();

        for batch in &batches {
            // Snapshot current outputs for template rendering (read-only during batch)
            let current_outputs = outputs.lock().await.clone();

            // Build one Tera context per step in the batch (before spawning tasks)
            let mut step_futures = Vec::new();
            for step_id in batch {
                let step = *step_map.get(step_id).unwrap();
                let step = step.clone();
                let task_str = task_input.to_string();
                let ctx_outputs = current_outputs.clone();
                let router = self.model_router.clone();
                let persona_provider = self.persona_provider.clone();

                step_futures.push(tokio::spawn(async move {
                    // 1. Render Tera prompt
                    let rendered = render_prompt(&step.prompt, &task_str, &ctx_outputs)
                        .map_err(|e| WorkflowError::TemplateError {
                            id: step.id.clone(),
                            reason: e.to_string(),
                        })?;

                    // 2. Resolve model: step.model > persona.preferred_model > "mock"
                    let model_name = if let Some(m) = &step.model {
                        m.clone()
                    } else if let Some(persona) = persona_provider.get(&step.agent) {
                        persona.preferred_model.clone()
                    } else {
                        "mock".to_string()
                    };

                    // 3. Call model provider
                    let req = CompletionRequest {
                        model: model_name.clone(),
                        messages: vec![Message { role: Role::User, content: rendered }],
                        max_tokens: None,
                        temperature: None,
                        system: None,
                    };
                    let resp = router.complete(req).await.map_err(|e| WorkflowError::StepFailed {
                        id: step.id.clone(),
                        reason: e.to_string(),
                    })?;

                    Ok::<(String, Option<String>, bool, String), WorkflowError>((
                        step.id.clone(),
                        step.output.clone(),
                        step.requires_approval.unwrap_or(false),
                        resp.content,
                    ))
                }));
            }

            // Await all tasks in this batch
            for fut in step_futures {
                let (step_id, output_name, needs_approval, content) = fut
                    .await
                    .map_err(|e| WorkflowError::StepFailed {
                        id: "unknown".to_string(),
                        reason: e.to_string(),
                    })??;

                // Handle approval gate (sequential after the step completes)
                let final_content = if needs_approval {
                    ApprovalGate::prompt(&step_id, &content).await?
                } else {
                    content
                };

                // Enforce budget if configured
                if let Some(budget) = &def.budget {
                    let token_estimate = estimate_tokens(&final_content);
                    let usd_estimate = token_estimate as f64 * 0.000_002; // $2 per 1M tokens placeholder
                    let mut used = budget_used.lock().await;
                    used.tokens += token_estimate;
                    used.estimated_usd += usd_estimate;
                    check_budget(budget, &used)?;
                }

                // Store output artifact
                if let Some(out_name) = output_name {
                    outputs.lock().await.insert(out_name, final_content);
                }

                steps_completed += 1;
            }
        }

        let final_outputs = Arc::try_unwrap(outputs)
            .unwrap_or_else(|a| {
                tokio::runtime::Handle::current()
                    .block_on(async { Mutex::new(a.lock().await.clone()) })
            })
            .into_inner();
        let final_budget = Arc::try_unwrap(budget_used)
            .unwrap_or_else(|a| {
                tokio::runtime::Handle::current()
                    .block_on(async { Mutex::new(a.lock().await.clone()) })
            })
            .into_inner();

        Ok(WorkflowResult {
            outputs: final_outputs,
            steps_completed,
            steps_total,
            budget_used: final_budget,
        })
    }
}

fn render_prompt(
    template: &str,
    task: &str,
    outputs: &HashMap<String, String>,
) -> Result<String, tera::Error> {
    let mut tera = tera::Tera::default();
    tera.add_raw_template("prompt", template)?;
    let mut ctx = tera::Context::new();
    ctx.insert("task", task);
    for (k, v) in outputs {
        ctx.insert(k, v);
    }
    tera.render("prompt", &ctx)
}

fn estimate_tokens(text: &str) -> u64 {
    // Rough approximation: 1 token ≈ 4 chars
    (text.len() as u64) / 4
}

fn check_budget(
    budget: &crate::types::BudgetConfig,
    used: &BudgetUsed,
) -> Result<(), WorkflowError> {
    let mode = budget.on_exceed.as_deref().unwrap_or("stop");

    let token_exceeded = budget.max_tokens_per_session
        .map_or(false, |max| used.tokens > max);
    let usd_exceeded = budget.max_usd_per_task
        .map_or(false, |max| used.estimated_usd > max);

    if !token_exceeded && !usd_exceeded {
        // Check alert threshold
        if let (Some(alert_pct), Some(max_tokens)) = (budget.alert_at_percent, budget.max_tokens_per_session) {
            let pct_used = (used.tokens as f64 / max_tokens as f64) * 100.0;
            if pct_used >= alert_pct as f64 {
                tracing::warn!(
                    "Budget alert: {:.0}% of token limit used ({}/{})",
                    pct_used, used.tokens, max_tokens
                );
            }
        }
        return Ok(());
    }

    let reason = if token_exceeded {
        format!("token limit {} exceeded (used {})", budget.max_tokens_per_session.unwrap(), used.tokens)
    } else {
        format!("USD limit ${:.6} exceeded (used ${:.6})", budget.max_usd_per_task.unwrap(), used.estimated_usd)
    };

    match mode {
        "alert-only" => {
            tracing::warn!("Budget exceeded (alert-only): {}", reason);
            Ok(())
        }
        "pause" => {
            // In pause mode: print to stderr and read y/n from stdin
            eprintln!("[BUDGET EXCEEDED] {} — continue? [y/n]: ", reason);
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            if input.trim().eq_ignore_ascii_case("y") {
                Ok(())
            } else {
                Err(WorkflowError::BudgetExceeded(reason))
            }
        }
        _ => Err(WorkflowError::BudgetExceeded(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StepDef, WorkflowDef, BudgetConfig};
    use agent007_models::{MockProvider, ModelProvider, ModelRouter};
    use agent007_core::dispatcher::LocalDispatcher;
    use agent007_core::persona::NoOpPersonaProvider;
    use std::sync::Arc;

    fn mock_runner(mock_reply: &str) -> WorkflowRunner {
        let mock = Arc::new(MockProvider::new(mock_reply, "mock"));
        let mut router = ModelRouter::new("mock");
        router.register("mock", mock as Arc<dyn ModelProvider>);
        let dispatcher = LocalDispatcher::new(32); // already Arc<LocalDispatcher>
        WorkflowRunner::new(
            Arc::new(NoOpPersonaProvider),
            Arc::new(router),
            dispatcher as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        )
    }

    fn simple_def() -> WorkflowDef {
        WorkflowDef {
            name: "simple".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "step1".to_string(),
                agent: "Researcher".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: "research {{task}}".to_string(),
                output: Some("notes".to_string()),
                requires_approval: None,
            }],
            budget: None,
        }
    }

    fn two_step_def() -> WorkflowDef {
        WorkflowDef {
            name: "two".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "step1".to_string(),
                    agent: "Researcher".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: None,
                    prompt: "research {{task}}".to_string(),
                    output: Some("notes".to_string()),
                    requires_approval: None,
                },
                StepDef {
                    id: "step2".to_string(),
                    agent: "Coder".to_string(),
                    model: None,
                    inputs: Some(vec!["notes".to_string()]),
                    depends_on: None,
                    prompt: "implement based on {{notes}}".to_string(),
                    output: Some("code".to_string()),
                    requires_approval: None,
                },
            ],
            budget: None,
        }
    }

    #[tokio::test]
    async fn run_single_step_returns_output() {
        let runner = mock_runner("mocked output");
        let result = runner.run(&simple_def(), "build auth").await.unwrap();
        assert_eq!(result.steps_completed, 1);
        assert_eq!(result.steps_total, 1);
        assert_eq!(result.outputs.get("notes").map(|s| s.as_str()), Some("mocked output"));
    }

    #[tokio::test]
    async fn run_two_step_pipeline_passes_artifact() {
        let runner = mock_runner("mocked reply");
        let result = runner.run(&two_step_def(), "add login").await.unwrap();
        assert_eq!(result.steps_completed, 2);
        assert!(result.outputs.contains_key("notes"));
        assert!(result.outputs.contains_key("code"));
    }

    #[tokio::test]
    async fn validate_cycle_returns_error() {
        let def = WorkflowDef {
            name: "cycle".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "a".to_string(), agent: "A".to_string(), model: None,
                    inputs: Some(vec!["out_b".to_string()]), depends_on: None,
                    prompt: "p".to_string(), output: Some("out_a".to_string()),
                    requires_approval: None,
                },
                StepDef {
                    id: "b".to_string(), agent: "B".to_string(), model: None,
                    inputs: Some(vec!["out_a".to_string()]), depends_on: None,
                    prompt: "p".to_string(), output: Some("out_b".to_string()),
                    requires_approval: None,
                },
            ],
            budget: None,
        };
        let runner = mock_runner("x");
        let err = runner.validate(&def).unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::CycleDetected));
    }

    #[tokio::test]
    async fn tera_task_variable_is_injected() {
        // The mock returns a fixed string, but we verify no TemplateError is returned.
        let runner = mock_runner("ok");
        let def = WorkflowDef {
            name: "t".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s".to_string(), agent: "A".to_string(), model: None,
                inputs: None, depends_on: None,
                prompt: "task is {{task}}".to_string(),
                output: None, requires_approval: None,
            }],
            budget: None,
        };
        runner.run(&def, "my task").await.unwrap();
    }

    #[tokio::test]
    async fn result_has_correct_steps_total() {
        let runner = mock_runner("r");
        let result = runner.run(&two_step_def(), "task").await.unwrap();
        assert_eq!(result.steps_total, 2);
    }

    #[tokio::test]
    async fn budget_token_limit_stops_run() {
        let runner = mock_runner("a very long output that is definitely more than 1 token");
        let def = WorkflowDef {
            name: "budget-test".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s1".to_string(), agent: "A".to_string(), model: None,
                inputs: None, depends_on: None,
                prompt: "do {{task}}".to_string(),
                output: Some("out".to_string()), requires_approval: None,
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: Some(1), // extremely low — 1 token
                max_usd_per_task: None,
                alert_at_percent: None,
                on_exceed: Some("stop".to_string()),
            }),
        };
        let err = runner.run(&def, "task").await.unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::BudgetExceeded(_)));
    }

    #[tokio::test]
    async fn budget_usd_limit_stops_run() {
        let runner = mock_runner("short");
        // Estimated cost of "short" (5 chars) = 5/4 * 0.000002 = 0.0000025 USD
        // Set limit just below that:
        let def = WorkflowDef {
            name: "budget-usd".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s1".to_string(), agent: "A".to_string(), model: None,
                inputs: None, depends_on: None,
                prompt: "do {{task}}".to_string(),
                output: Some("out".to_string()), requires_approval: None,
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: None,
                max_usd_per_task: Some(0.000_000_001), // sub-nano USD — always exceeded
                alert_at_percent: None,
                on_exceed: Some("stop".to_string()),
            }),
        };
        let err = runner.run(&def, "task").await.unwrap_err();
        assert!(matches!(err, crate::error::WorkflowError::BudgetExceeded(_)));
    }

    #[tokio::test]
    async fn budget_alert_only_does_not_stop_run() {
        let runner = mock_runner("a very long output that is definitely more than 1 token");
        let def = WorkflowDef {
            name: "budget-alert".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s1".to_string(), agent: "A".to_string(), model: None,
                inputs: None, depends_on: None,
                prompt: "do {{task}}".to_string(),
                output: Some("out".to_string()), requires_approval: None,
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: Some(1), // would exceed but mode is alert-only
                max_usd_per_task: None,
                alert_at_percent: None,
                on_exceed: Some("alert-only".to_string()),
            }),
        };
        // Should succeed despite exceeding the token limit
        let result = runner.run(&def, "task").await.unwrap();
        assert_eq!(result.steps_completed, 1);
    }
}
