use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WorkflowDef {
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<StepDef>,
    pub budget: Option<BudgetConfig>,
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

#[derive(Deserialize, Serialize, Debug, Clone)]
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
    #[serde(default, rename = "type")]
    pub r#type: StepType,
    pub evaluate: Option<EvaluateConfig>,
    pub routes: Option<Vec<RouteConfig>>,
    /// For sub-workflow steps: the name of the workflow file to call (without .toml extension).
    pub workflow: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct BudgetConfig {
    pub max_tokens_per_session: Option<u64>,
    pub max_usd_per_task: Option<f64>,
    pub alert_at_percent: Option<u8>,
    pub on_exceed: Option<String>, // "pause" | "stop" | "alert-only"
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
}
