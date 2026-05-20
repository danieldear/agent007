use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WorkflowDef {
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<StepDef>,
    pub budget: Option<BudgetConfig>,
    pub reliability: Option<ReliabilityConfig>,
    pub eval_gate: Option<EvalGateConfig>,
}

impl WorkflowDef {
    /// Validate the workflow definition's schema: required fields, type-specific constraints.
    /// This runs before DAG validation and catches authoring mistakes early.
    pub fn validate_schema(&self) -> Result<(), crate::error::WorkflowError> {
        if self.name.trim().is_empty() {
            return Err(crate::error::WorkflowError::SchemaError {
                reason: "workflow 'name' must not be empty".to_string(),
            });
        }
        if self.steps.is_empty() {
            return Err(crate::error::WorkflowError::SchemaError {
                reason: "workflow must have at least one step".to_string(),
            });
        }
        for step in &self.steps {
            if step.id.trim().is_empty() {
                return Err(crate::error::WorkflowError::SchemaError {
                    reason: "every step must have a non-empty 'id'".to_string(),
                });
            }
            if step.agent.trim().is_empty() {
                return Err(crate::error::WorkflowError::SchemaError {
                    reason: format!("step '{}': 'agent' must not be empty", step.id),
                });
            }
            match step.r#type {
                StepType::SubWorkflow => {
                    if step.workflow.is_none() {
                        return Err(crate::error::WorkflowError::SchemaError {
                            reason: format!(
                                "step '{}': sub-workflow steps must specify 'workflow'",
                                step.id
                            ),
                        });
                    }
                }
                StepType::Evaluator => {
                    if step.evaluate.is_none() {
                        return Err(crate::error::WorkflowError::SchemaError {
                            reason: format!(
                                "step '{}': evaluator steps must have an 'evaluate' block",
                                step.id
                            ),
                        });
                    }
                }
                StepType::Router => {
                    if step.routes.as_ref().map_or(true, |r| r.is_empty()) {
                        return Err(crate::error::WorkflowError::SchemaError {
                            reason: format!(
                                "step '{}': router steps must have at least one 'routes' entry",
                                step.id
                            ),
                        });
                    }
                }
                StepType::Execute => {
                    if step.prompt.is_none() && step.skill.is_none() {
                        return Err(crate::error::WorkflowError::SchemaError {
                            reason: format!(
                                "step '{}': must specify either 'prompt' or 'skill'",
                                step.id
                            ),
                        });
                    }
                }
                StepType::Extract => {
                    if step.extract.is_none() {
                        return Err(crate::error::WorkflowError::SchemaError {
                            reason: format!(
                                "step '{}': extract steps must have an 'extract' block",
                                step.id
                            ),
                        });
                    }
                }
                StepType::MultiAgent => {
                    if step.workers.as_ref().map_or(true, |w| w.is_empty()) {
                        return Err(crate::error::WorkflowError::SchemaError {
                            reason: format!(
                                "step '{}': multi-agent steps must have at least one worker in 'workers'",
                                step.id
                            ),
                        });
                    }
                    // Validate that every worker has a non-empty persona name.
                    for wc in step.workers.as_ref().unwrap() {
                        if wc.persona.trim().is_empty() {
                            return Err(crate::error::WorkflowError::SchemaError {
                                reason: format!(
                                    "step '{}': every worker entry must have a non-empty 'persona'",
                                    step.id
                                ),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StepType {
    #[default]
    Execute,
    Evaluator,
    Router,
    /// Call another workflow by name and inject its outputs into the current context.
    SubWorkflow,
    /// Run a deterministic ETR tool call — no LLM round-trip.
    Extract,
    /// Fan out to multiple worker agents (persona-based), then combine their outputs.
    /// The step's `agent` field names the orchestrator persona.
    /// Worker details are declared in the `workers` field.
    MultiAgent,
}

/// Execution mode for a worker within a multi-agent step.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WorkerRunMode {
    /// Run this worker concurrently with other parallel workers (default).
    #[default]
    Parallel,
    /// Run this worker after all parallel workers in this step have completed.
    /// Receives parallel workers' combined output as context.
    Sequential,
}

/// Configuration for a single worker within a `multi-agent` step.
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct WorkerConfig {
    /// Persona name to use for this worker (looked up in the PersonaRegistry).
    pub persona: String,
    /// Skill trigger names to inject into this worker's system prompt for this
    /// invocation. These are *per-invocation* additions; the persona's own
    /// `skills` field provides always-on defaults.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Execution mode: `parallel` (default) or `sequential`.
    #[serde(default)]
    pub run: WorkerRunMode,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ExtractConfig {
    /// ETR tool name (e.g. "etr.grep", "etr.csv_slice")
    pub tool: String,
    /// Input JSON for the ETR tool call
    pub input: serde_json::Value,
    /// Whether to compact the output (default true)
    #[serde(default = "default_true")]
    pub compact: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct EvaluateConfig {
    pub condition: Option<String>,
    pub decision_field: Option<String>,
    pub on_pass: String,
    pub on_fail: String,
    pub max_retries: Option<u32>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RouteConfig {
    pub when: Option<String>,
    pub goto: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct StepDef {
    pub id: String,
    pub agent: String,
    pub model: Option<String>,
    pub inputs: Option<Vec<String>>,
    pub depends_on: Option<Vec<String>>,
    pub prompt: Option<String>,
    pub skill: Option<String>,
    pub output: Option<String>,
    pub requires_approval: Option<bool>,
    /// Optional prompt shown to the user at the approval gate. If set, the AI must
    /// present this message verbatim when asking the user for their approve/deny/edit decision.
    pub approval_prompt: Option<String>,
    #[serde(default, rename = "type")]
    pub r#type: StepType,
    pub evaluate: Option<EvaluateConfig>,
    pub routes: Option<Vec<RouteConfig>>,
    /// For sub-workflow steps: the name of the workflow file to call (without .toml extension).
    pub workflow: Option<String>,
    /// For `type: extract` steps: the ETR tool call configuration.
    pub extract: Option<ExtractConfig>,
    /// If true, cache this step's output by content hash and skip re-execution on cache hit.
    #[serde(default)]
    pub cache: bool,
    /// Worker configurations for `type: multi-agent` steps.
    /// Ignored for all other step types.
    #[serde(default)]
    pub workers: Option<Vec<WorkerConfig>>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct BudgetConfig {
    pub max_tokens_per_session: Option<u64>,
    pub max_usd_per_task: Option<f64>,
    pub alert_at_percent: Option<u8>,
    pub on_exceed: Option<String>, // "pause" | "stop" | "alert-only"
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ReliabilityConfig {
    pub enabled: Option<bool>,
    pub recovery: Option<ReliabilityRecoveryConfig>,
    pub budget_governor: Option<ReliabilityBudgetGovernorConfig>,
    pub guardrails: Option<ReliabilityGuardrailsConfig>,
    pub confidence: Option<ReliabilityConfidenceConfig>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ReliabilityRecoveryConfig {
    pub enabled: Option<bool>,
    pub max_step_retries: Option<u32>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ReliabilityBudgetGovernorConfig {
    pub enabled: Option<bool>,
    pub max_degradations_per_run: Option<u32>,
    pub degrade_output_chars: Option<usize>,
    /// Optional token threshold for lazy injection stubbing in hosted workflows.
    pub lazy_injection_threshold: Option<usize>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ReliabilityGuardrailsConfig {
    pub enabled: Option<bool>,
    pub terms: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ReliabilityConfidenceConfig {
    pub enabled: Option<bool>,
    pub low_terms: Option<Vec<String>>,
    pub missing_requires_approval: Option<bool>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct EvalGateConfig {
    pub enabled: Option<bool>,
    pub release_class: Option<bool>,
    pub mode: Option<EvalGateMode>,
    pub baseline_window: Option<usize>,
    pub min_baseline_runs: Option<usize>,
    pub thresholds: Option<EvalGateThresholdConfig>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EvalGateMode {
    #[default]
    FailOpen,
    FailClosed,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct EvalGateThresholdConfig {
    pub max_quality_score_drop: Option<f64>,
    pub max_cost_usd_increase: Option<f64>,
    pub max_latency_ms_increase: Option<f64>,
    pub max_retry_increase: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
pub struct WorkflowResult {
    pub outputs: HashMap<String, String>,
    pub steps_completed: usize,
    pub steps_total: usize,
    pub budget_used: BudgetUsed,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BudgetUsed {
    pub tokens: u64,
    pub estimated_usd: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_TOML: &str = r#"
name = "Test Workflow"

[[steps]]
id = "step1"
agent = "Researcher"
prompt = "Research {{task}}"
output = "notes"
"#;

    const FULL_TOML: &str = r#"
name = "TDD Feature Development"
description = "Research → Architect → Coder"

[[steps]]
id = "research"
agent = "Researcher"
model = "claude"
prompt = "Research best practices for: {{task}}"
output = "research_notes"

[[steps]]
id = "architect"
agent = "Architect"
model = "claude"
inputs = ["research_notes"]
prompt = "Design an implementation plan"
output = "plan"
requires_approval = true

[[steps]]
id = "implement"
agent = "Coder"
model = "codex"
inputs = ["plan"]
depends_on = ["architect"]
prompt = "Implement until all tests pass"
output = "implementation"

[budget]
max_tokens_per_session = 500000
max_usd_per_task = 2.00
alert_at_percent = 80
on_exceed = "pause"
"#;

    const EVALUATOR_YAML: &str = r#"
name = "Eval Test"

[[steps]]
id = "impl"
agent = "Coder"
prompt = "code {{task}}"
output = "code"

[[steps]]
id = "review"
agent = "Reviewer"
type = "evaluator"
prompt = "review {{code}}"
output = "verdict"

[steps.evaluate]
decision_field = "verdict"
on_pass = "done"
on_fail = "impl"
max_retries = 3
"#;

    const ROUTER_YAML: &str = r#"
name = "Router Test"

[[steps]]
id = "classify"
agent = "Router"
type = "router"
prompt = "classify {{task}}"
output = "route"

[[steps.routes]]
when = "frontend"
goto = "ui"

[[steps.routes]]
goto = "api"
default = true
"#;

    #[test]
    fn deserialize_minimal_workflow() {
        let def: WorkflowDef = toml::from_str(MINIMAL_TOML).unwrap();
        assert_eq!(def.name, "Test Workflow");
        assert_eq!(def.steps.len(), 1);
        assert_eq!(def.steps[0].id, "step1");
        assert_eq!(def.steps[0].agent, "Researcher");
        assert!(def.budget.is_none());
    }

    #[test]
    fn deserialize_full_workflow() {
        let def: WorkflowDef = toml::from_str(FULL_TOML).unwrap();
        assert_eq!(def.name, "TDD Feature Development");
        assert_eq!(def.steps.len(), 3);
        let architect = &def.steps[1];
        assert_eq!(architect.id, "architect");
        assert_eq!(architect.requires_approval, Some(true));
        assert_eq!(architect.inputs.as_ref().unwrap(), &["research_notes"]);
        let budget = def.budget.as_ref().unwrap();
        assert_eq!(budget.max_tokens_per_session, Some(500_000));
        assert_eq!(budget.on_exceed.as_deref(), Some("pause"));
    }

    #[test]
    fn step_optional_fields_default_to_none() {
        let def: WorkflowDef = toml::from_str(MINIMAL_TOML).unwrap();
        let step = &def.steps[0];
        assert!(step.model.is_none());
        assert!(step.inputs.is_none());
        assert!(step.depends_on.is_none());
        assert!(step.requires_approval.is_none());
    }

    #[test]
    fn deserialize_evaluator_step() {
        let def: WorkflowDef = toml::from_str(EVALUATOR_YAML).unwrap();
        assert_eq!(def.steps.len(), 2);
        let review = &def.steps[1];
        assert_eq!(review.r#type, StepType::Evaluator);
        let eval = review.evaluate.as_ref().unwrap();
        assert_eq!(eval.on_pass, "done");
        assert_eq!(eval.on_fail, "impl");
        assert_eq!(eval.max_retries, Some(3));
        assert_eq!(eval.decision_field.as_deref(), Some("verdict"));
        assert!(eval.condition.is_none());
    }

    #[test]
    fn deserialize_router_step() {
        let def: WorkflowDef = toml::from_str(ROUTER_YAML).unwrap();
        let classify = &def.steps[0];
        assert_eq!(classify.r#type, StepType::Router);
        let routes = classify.routes.as_ref().unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].when.as_deref(), Some("frontend"));
        assert_eq!(routes[0].goto, "ui");
        assert!(!routes[0].default);
        assert!(routes[1].when.is_none());
        assert_eq!(routes[1].goto, "api");
        assert!(routes[1].default);
    }

    #[test]
    fn existing_workflow_without_type_defaults_to_execute() {
        let def: WorkflowDef = toml::from_str(MINIMAL_TOML).unwrap();
        assert_eq!(def.steps[0].r#type, StepType::Execute);
        assert!(def.steps[0].evaluate.is_none());
        assert!(def.steps[0].routes.is_none());
    }

    #[test]
    fn deserialize_step_with_skill() {
        let yaml = r#"
name = "Skill Test"

[[steps]]
id = "design"
agent = "Architect"
skill = "/dev-architect"
output = "design"
"#;
        let def: WorkflowDef = toml::from_str(yaml).unwrap();
        assert_eq!(def.steps[0].skill.as_deref(), Some("/dev-architect"));
        assert!(def.steps[0].prompt.is_none());
    }

    #[test]
    fn deserialize_multi_agent_step() {
        let toml_str = r#"
name = "Multi-Agent Test"

[[steps]]
id = "investigate"
type = "multi-agent"
agent = "tech-lead"
prompt = "{{task}}"
output = "investigation_result"

[[steps.workers]]
persona = "debugger"
skills = ["wifi-debug"]
run = "parallel"

[[steps.workers]]
persona = "coder"
skills = ["wifi-driver-codebase"]
run = "parallel"

[[steps.workers]]
persona = "reporter"
run = "sequential"
"#;
        let def: WorkflowDef = toml::from_str(toml_str).unwrap();
        assert_eq!(def.steps.len(), 1);
        let step = &def.steps[0];
        assert_eq!(step.r#type, StepType::MultiAgent);
        assert_eq!(step.agent, "tech-lead");

        let workers = step.workers.as_ref().unwrap();
        assert_eq!(workers.len(), 3);
        assert_eq!(workers[0].persona, "debugger");
        assert_eq!(workers[0].skills, vec!["wifi-debug"]);
        assert_eq!(workers[0].run, WorkerRunMode::Parallel);
        assert_eq!(workers[1].persona, "coder");
        assert_eq!(workers[2].persona, "reporter");
        assert_eq!(workers[2].run, WorkerRunMode::Sequential);
    }

    #[test]
    fn multi_agent_step_without_workers_fails_validation() {
        let toml_str = r#"
name = "Invalid Multi-Agent"

[[steps]]
id = "bad"
type = "multi-agent"
agent = "tech-lead"
prompt = "{{task}}"
"#;
        let def: WorkflowDef = toml::from_str(toml_str).unwrap();
        let result = def.validate_schema();
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("multi-agent"), "error should mention multi-agent: {msg}");
        assert!(msg.contains("bad"), "error should mention step id: {msg}");
    }

    #[test]
    fn worker_run_mode_defaults_to_parallel() {
        let toml_str = r#"
name = "Default Run Test"

[[steps]]
id = "step1"
type = "multi-agent"
agent = "orch"
prompt = "go"

[[steps.workers]]
persona = "worker-a"
"#;
        let def: WorkflowDef = toml::from_str(toml_str).unwrap();
        let worker = &def.steps[0].workers.as_ref().unwrap()[0];
        assert_eq!(worker.run, WorkerRunMode::Parallel);
        assert!(worker.skills.is_empty());
    }

    #[test]
    fn existing_step_without_workers_field_still_parses() {
        // Regression: existing workflows without 'workers' must still parse cleanly
        let def: WorkflowDef = toml::from_str(MINIMAL_TOML).unwrap();
        assert!(def.steps[0].workers.is_none());
    }

    #[test]
    fn worker_with_empty_persona_fails_validation() {
        let toml_str = r#"
name = "Bad Worker"

[[steps]]
id = "analyse"
type = "multi-agent"
agent = "lead"
prompt = "go"

[[steps.workers]]
persona = ""
"#;
        let def: WorkflowDef = toml::from_str(toml_str).unwrap();
        let result = def.validate_schema();
        assert!(result.is_err(), "empty persona should fail validation");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("non-empty"),
            "error should mention non-empty requirement: {msg}"
        );
        assert!(msg.contains("analyse"), "error should name the step: {msg}");
    }

    #[test]
    fn worker_with_whitespace_only_persona_fails_validation() {
        let toml_str = r#"
name = "Whitespace Persona"

[[steps]]
id = "compute"
type = "multi-agent"
agent = "lead"
prompt = "go"

[[steps.workers]]
persona = "   "
"#;
        let def: WorkflowDef = toml::from_str(toml_str).unwrap();
        let result = def.validate_schema();
        assert!(result.is_err(), "whitespace-only persona should fail validation");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("non-empty"),
            "error should mention non-empty requirement: {msg}"
        );
    }

    #[test]
    fn worker_skills_default_to_empty_when_absent() {
        let toml_str = r#"
name = "No Skills"

[[steps]]
id = "step1"
type = "multi-agent"
agent = "orch"
prompt = "go"

[[steps.workers]]
persona = "analyst"
"#;
        let def: WorkflowDef = toml::from_str(toml_str).unwrap();
        let worker = &def.steps[0].workers.as_ref().unwrap()[0];
        assert!(
            worker.skills.is_empty(),
            "skills should default to empty vec when not specified"
        );
    }

    #[test]
    fn worker_skills_round_trip_multiple_values() {
        let toml_str = r#"
name = "With Skills"

[[steps]]
id = "step1"
type = "multi-agent"
agent = "orch"
prompt = "go"

[[steps.workers]]
persona = "coder"
skills = ["dev-debug", "code-review", "style-guide"]
"#;
        let def: WorkflowDef = toml::from_str(toml_str).unwrap();
        let worker = &def.steps[0].workers.as_ref().unwrap()[0];
        assert_eq!(
            worker.skills,
            vec!["dev-debug", "code-review", "style-guide"]
        );
    }
}
