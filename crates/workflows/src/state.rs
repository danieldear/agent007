use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::approval::ApprovalDecision;
use crate::types::{BudgetUsed, StepDef, WorkflowDef};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowRunStatus {
    #[default]
    Running,
    WaitingApproval,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowStepStatus {
    #[default]
    Pending,
    Running,
    AwaitingApproval,
    Completed,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunRequest {
    pub workflow: String,
    pub task: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSourceRef {
    pub workflow_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub step_id: String,
    pub agent: String,
    pub output_key: Option<String>,
    pub content: String,
    pub content_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStepState {
    pub id: String,
    pub agent: String,
    pub status: WorkflowStepStatus,
    pub attempts: u32,
    pub output_key: Option<String>,
    pub output_preview: Option<String>,
    pub selected_route: Option<String>,
    pub selected_target: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunState {
    pub workflow: String,
    pub task: String,
    pub status: WorkflowRunStatus,
    pub steps_total: usize,
    pub steps_completed: usize,
    pub completed_steps: Vec<String>,
    pub skipped_steps: Vec<String>,
    pub retry_counts: HashMap<String, u32>,
    pub outputs: HashMap<String, String>,
    pub budget_used: BudgetUsed,
    pub steps: Vec<WorkflowStepState>,
    pub pending_approval: Option<PendingApproval>,
    pub approval_decisions: HashMap<String, ApprovalDecision>,
    pub last_error: Option<String>,
}

impl WorkflowRunState {
    pub fn new(def: &WorkflowDef, task: &str) -> Self {
        Self {
            workflow: def.name.clone(),
            task: task.to_string(),
            status: WorkflowRunStatus::Running,
            steps_total: def.steps.len(),
            steps_completed: 0,
            completed_steps: Vec::new(),
            skipped_steps: Vec::new(),
            retry_counts: HashMap::new(),
            outputs: HashMap::new(),
            budget_used: BudgetUsed::default(),
            steps: def.steps.iter().map(WorkflowStepState::from).collect(),
            pending_approval: None,
            approval_decisions: HashMap::new(),
            last_error: None,
        }
    }

    pub fn request(&self) -> WorkflowRunRequest {
        WorkflowRunRequest {
            workflow: self.workflow.clone(),
            task: self.task.clone(),
        }
    }

    pub fn mark_step_running(&mut self, step: &StepDef) {
        let attempts = self.retry_counts.get(&step.id).copied().unwrap_or(0) + 1;
        let step_state = self.step_mut(&step.id);
        step_state.status = WorkflowStepStatus::Running;
        step_state.attempts = attempts;
        step_state.error = None;
    }

    pub fn mark_step_completed(&mut self, step: &StepDef, output: &str) {
        let output_preview = preview(output);
        let step_state = self.step_mut(&step.id);
        step_state.status = WorkflowStepStatus::Completed;
        step_state.output_key = step.output.clone();
        step_state.output_preview = Some(output_preview);
        step_state.error = None;

        if !self.completed_steps.iter().any(|id| id == &step.id) {
            self.completed_steps.push(step.id.clone());
        }
        self.completed_steps.sort();
        self.completed_steps.dedup();
        self.steps_completed = self.completed_steps.len();
        self.last_error = None;
    }

    pub fn mark_step_awaiting_approval(&mut self, step: &StepDef, content: &str) {
        self.status = WorkflowRunStatus::WaitingApproval;
        self.last_error = None;
        self.pending_approval = Some(PendingApproval {
            step_id: step.id.clone(),
            agent: step.agent.clone(),
            output_key: step.output.clone(),
            content: content.to_string(),
            content_preview: preview(content),
        });
        let step_state = self.step_mut(&step.id);
        step_state.status = WorkflowStepStatus::AwaitingApproval;
        step_state.output_key = step.output.clone();
        step_state.output_preview = Some(preview(content));
        step_state.error = None;
    }

    pub fn clear_pending_approval(&mut self) {
        self.pending_approval = None;
        if self.status == WorkflowRunStatus::WaitingApproval {
            self.status = WorkflowRunStatus::Running;
        }
    }

    pub fn record_approval_decision(
        &mut self,
        step_id: &str,
        mut decision: ApprovalDecision,
    ) {
        if decision.content.is_none() {
            if let Some(pending) = self
                .pending_approval
                .as_ref()
                .filter(|pending| pending.step_id == step_id)
            {
                decision.content = Some(pending.content.clone());
            }
        }
        self.approval_decisions.insert(step_id.to_string(), decision);
        if self
            .pending_approval
            .as_ref()
            .map(|pending| pending.step_id.as_str() == step_id)
            .unwrap_or(false)
        {
            self.pending_approval = None;
        }
        if self.status == WorkflowRunStatus::WaitingApproval {
            self.status = WorkflowRunStatus::Running;
        }
    }

    pub fn approval_decision(&self, step_id: &str) -> Option<ApprovalDecision> {
        self.approval_decisions.get(step_id).cloned()
    }

    pub fn mark_step_retry(&mut self, step_id: &str, attempt: u32) {
        self.retry_counts.insert(step_id.to_string(), attempt);
        let step_state = self.step_mut(step_id);
        step_state.attempts = attempt;
        step_state.status = WorkflowStepStatus::Pending;
    }

    pub fn mark_step_skipped(&mut self, step_id: &str) {
        if !self.skipped_steps.iter().any(|id| id == step_id) {
            self.skipped_steps.push(step_id.to_string());
            self.skipped_steps.sort();
        }
        let step_state = self.step_mut(step_id);
        step_state.status = WorkflowStepStatus::Skipped;
        step_state.error = None;
    }

    pub fn mark_route_selected(&mut self, step_id: &str, selected: &str) {
        let step_state = self.step_mut(step_id);
        step_state.selected_route = Some(selected.to_string());
    }

    pub fn mark_step_target(&mut self, step_id: &str, selected: &str) {
        let step_state = self.step_mut(step_id);
        step_state.selected_target = Some(selected.to_string());
    }

    pub fn reset_step(&mut self, step_id: &str) {
        let attempts = self.retry_counts.get(step_id).copied().unwrap_or(0);
        let step_state = self.step_mut(step_id);
        step_state.status = WorkflowStepStatus::Pending;
        step_state.attempts = attempts;
        step_state.output_preview = None;
        step_state.selected_route = None;
        step_state.selected_target = None;
        step_state.error = None;
    }

    pub fn sync_outputs(&mut self, outputs: HashMap<String, String>) {
        self.outputs = outputs;
    }

    pub fn sync_budget(&mut self, budget_used: BudgetUsed) {
        self.budget_used = budget_used;
    }

    pub fn mark_failed(&mut self, step_id: Option<&str>, error: impl Into<String>) {
        let error = error.into();
        self.status = WorkflowRunStatus::Failed;
        self.last_error = Some(error.clone());
        if let Some(step_id) = step_id {
            let step_state = self.step_mut(step_id);
            step_state.status = WorkflowStepStatus::Failed;
            step_state.error = Some(error);
        }
    }

    pub fn mark_succeeded(&mut self) {
        self.status = WorkflowRunStatus::Succeeded;
        self.pending_approval = None;
        self.last_error = None;
        self.steps_completed = self.completed_steps.len();
    }

    fn step_mut(&mut self, step_id: &str) -> &mut WorkflowStepState {
        self.steps
            .iter_mut()
            .find(|step| step.id == step_id)
            .expect("workflow state is missing a step definition")
    }
}

impl From<&StepDef> for WorkflowStepState {
    fn from(step: &StepDef) -> Self {
        Self {
            id: step.id.clone(),
            agent: step.agent.clone(),
            status: WorkflowStepStatus::Pending,
            attempts: 0,
            output_key: step.output.clone(),
            output_preview: None,
            selected_route: None,
            selected_target: None,
            error: None,
        }
    }
}

fn preview(text: &str) -> String {
    const MAX_CHARS: usize = 160;
    let preview: String = text.chars().take(MAX_CHARS).collect();
    if text.chars().count() > MAX_CHARS {
        format!("{preview}...")
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StepType, WorkflowDef};

    fn sample_workflow() -> WorkflowDef {
        WorkflowDef {
            name: "sample".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "research".to_string(),
                agent: "Researcher".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("research {{task}}".to_string()),
                skill: None,
                output: Some("notes".to_string()),
                requires_approval: None,
                r#type: StepType::Execute,
                evaluate: None,
                routes: None,
            }],
            budget: None,
        }
    }

    #[test]
    fn new_state_tracks_all_steps() {
        let workflow = sample_workflow();
        let state = WorkflowRunState::new(&workflow, "build auth");
        assert_eq!(state.workflow, "sample");
        assert_eq!(state.steps_total, 1);
        assert_eq!(state.steps[0].status, WorkflowStepStatus::Pending);
    }

    #[test]
    fn completing_step_updates_summary() {
        let workflow = sample_workflow();
        let step = workflow.steps[0].clone();
        let mut state = WorkflowRunState::new(&workflow, "build auth");
        state.mark_step_running(&step);
        state.mark_step_completed(&step, "done");
        state.mark_succeeded();

        assert_eq!(state.status, WorkflowRunStatus::Succeeded);
        assert_eq!(state.steps_completed, 1);
        assert_eq!(state.completed_steps, vec!["research"]);
        assert_eq!(state.steps[0].status, WorkflowStepStatus::Completed);
    }
}
