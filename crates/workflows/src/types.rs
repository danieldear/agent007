use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Debug, Clone)]
pub struct WorkflowDef {
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<StepDef>,
    pub budget: Option<BudgetConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct StepDef {
    pub id: String,
    pub agent: String,
    pub model: Option<String>,
    pub inputs: Option<Vec<String>>,
    pub depends_on: Option<Vec<String>>,
    pub prompt: String,
    pub output: Option<String>,
    pub requires_approval: Option<bool>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct BudgetConfig {
    pub max_tokens_per_session: Option<u64>,
    pub max_usd_per_task: Option<f64>,
    pub alert_at_percent: Option<u8>,
    pub on_exceed: Option<String>, // "pause" | "stop" | "alert-only"
}

#[derive(Debug, Default)]
pub struct WorkflowResult {
    pub outputs: HashMap<String, String>,
    pub steps_completed: usize,
    pub steps_total: usize,
    pub budget_used: BudgetUsed,
}

#[derive(Debug, Default, Clone)]
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
}
