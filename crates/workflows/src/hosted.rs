use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use agent007_core::persona::PersonaProvider;

use crate::approval::{ApprovalDecision, ApprovalDecisionKind};
use crate::error::WorkflowError;
use crate::runner::{
    build_step_dependents, check_budget, estimate_tokens, evaluate_condition,
    evaluate_decision_field, match_route, render_prompt, reset_steps_from_target, step_is_ready,
};
use crate::state::{PendingApproval, WorkflowRunState, WorkflowRunStatus, WorkflowStepStatus};
use crate::types::{StepDef, StepType, WorkflowDef};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HostedWorkflowProgressStatus {
    Ready,
    AwaitingOutputs,
    AwaitingApproval,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedWorkflowStep {
    pub id: String,
    pub agent: String,
    pub model_hint: String,
    pub system_prompt: Option<String>,
    pub prompt: String,
    pub output_key: Option<String>,
    pub inputs: HashMap<String, String>,
    pub depends_on: Vec<String>,
    pub step_type: StepType,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedWorkflowProgress {
    pub workflow: String,
    pub task: String,
    pub status: HostedWorkflowProgressStatus,
    pub ready_steps: Vec<HostedWorkflowStep>,
    pub running_steps: Vec<String>,
    pub completed_steps: usize,
    pub total_steps: usize,
    pub outputs_available: Vec<String>,
    pub pending_approval: Option<PendingApproval>,
    pub last_error: Option<String>,
    pub message: Option<String>,
}

pub struct HostedWorkflowEngine {
    persona_provider: Arc<dyn PersonaProvider>,
}

impl HostedWorkflowEngine {
    pub fn new(persona_provider: Arc<dyn PersonaProvider>) -> Self {
        Self { persona_provider }
    }

    pub fn status(
        &self,
        def: &WorkflowDef,
        state: &mut WorkflowRunState,
    ) -> Result<HostedWorkflowProgress, WorkflowError> {
        self.finalize_approved_steps(def, state)?;
        self.progress(def, state, false)
    }

    pub fn dispatch(
        &self,
        def: &WorkflowDef,
        state: &mut WorkflowRunState,
    ) -> Result<HostedWorkflowProgress, WorkflowError> {
        self.finalize_approved_steps(def, state)?;
        self.progress(def, state, true)
    }

    pub fn submit_step_output(
        &self,
        def: &WorkflowDef,
        state: &mut WorkflowRunState,
        step_id: &str,
        output: &str,
    ) -> Result<HostedWorkflowProgress, WorkflowError> {
        self.finalize_approved_steps(def, state)?;

        if state.status == WorkflowRunStatus::Failed {
            return Ok(self.failed_progress(state, "workflow is already failed"));
        }
        if state.status == WorkflowRunStatus::Succeeded {
            return Ok(self.succeeded_progress(state, "workflow is already complete"));
        }
        if state.pending_approval.is_some() {
            return Ok(self.awaiting_approval_progress(state, "approval is still pending"));
        }

        let step_map: HashMap<String, StepDef> = def
            .steps
            .iter()
            .map(|step| (step.id.clone(), step.clone()))
            .collect();
        let step = step_map
            .get(step_id)
            .ok_or_else(|| WorkflowError::StepFailed {
                id: step_id.to_string(),
                reason: "unknown workflow step".to_string(),
            })?;

        let step_state = state
            .steps
            .iter()
            .find(|candidate| candidate.id == step_id)
            .ok_or_else(|| WorkflowError::StepFailed {
                id: step_id.to_string(),
                reason: "workflow state is missing the requested step".to_string(),
            })?;

        match step_state.status {
            WorkflowStepStatus::Completed | WorkflowStepStatus::Skipped => {
                return Err(WorkflowError::StepFailed {
                    id: step_id.to_string(),
                    reason: format!(
                        "step '{}' is already {}",
                        step_id,
                        match step_state.status {
                            WorkflowStepStatus::Completed => "completed",
                            WorkflowStepStatus::Skipped => "skipped",
                            _ => unreachable!(),
                        }
                    ),
                });
            }
            WorkflowStepStatus::AwaitingApproval => {
                return Ok(self.awaiting_approval_progress(
                    state,
                    format!("step '{}' is waiting for approval", step_id),
                ));
            }
            WorkflowStepStatus::Failed => {
                return Ok(self.failed_progress(
                    state,
                    format!("step '{}' is already marked failed", step_id),
                ));
            }
            WorkflowStepStatus::Pending => {
                let completed = state
                    .completed_steps
                    .iter()
                    .cloned()
                    .collect::<HashSet<_>>();
                if !step_is_ready(step, &state.outputs, &completed) {
                    return Err(WorkflowError::StepFailed {
                        id: step_id.to_string(),
                        reason: "step is not ready yet".to_string(),
                    });
                }
                state.mark_step_running(step);
            }
            WorkflowStepStatus::Running => {}
        }

        let final_content = match self.resolve_submission_content(state, step, output) {
            SubmissionResolution::AwaitingApproval(message) => {
                return Ok(self.awaiting_approval_progress(state, message));
            }
            SubmissionResolution::Content(content) => content,
            SubmissionResolution::Failed(message) => {
                state.mark_failed(Some(step_id), message.clone());
                return Ok(self.failed_progress(state, message));
            }
        };

        self.complete_step(def, state, step, &final_content);
        self.dispatch(def, state)
    }

    fn finalize_approved_steps(
        &self,
        def: &WorkflowDef,
        state: &mut WorkflowRunState,
    ) -> Result<(), WorkflowError> {
        let step_map: HashMap<String, StepDef> = def
            .steps
            .iter()
            .map(|step| (step.id.clone(), step.clone()))
            .collect();

        loop {
            let awaiting_step = state
                .steps
                .iter()
                .find(|step| {
                    step.status == WorkflowStepStatus::AwaitingApproval
                        && state.approval_decision(&step.id).is_some()
                })
                .map(|step| step.id.clone());

            let Some(step_id) = awaiting_step else {
                break;
            };
            let step = step_map
                .get(&step_id)
                .ok_or_else(|| WorkflowError::StepFailed {
                    id: step_id.clone(),
                    reason: "workflow state references an unknown step".to_string(),
                })?;
            let decision =
                state
                    .approval_decision(&step_id)
                    .ok_or_else(|| WorkflowError::StepFailed {
                        id: step_id.clone(),
                        reason: "approval decision is missing".to_string(),
                    })?;
            match decision.decision {
                ApprovalDecisionKind::Approve | ApprovalDecisionKind::Edit => {
                    let content =
                        decision
                            .content
                            .clone()
                            .ok_or_else(|| WorkflowError::StepFailed {
                                id: step_id.clone(),
                                reason: "approval decision is missing content".to_string(),
                            })?;
                    self.complete_step(def, state, step, &content);
                }
                ApprovalDecisionKind::Deny => {
                    state.mark_failed(
                        Some(&step_id),
                        WorkflowError::ApprovalDenied(step_id.clone()).to_string(),
                    );
                    break;
                }
            }
        }
        Ok(())
    }

    fn progress(
        &self,
        def: &WorkflowDef,
        state: &mut WorkflowRunState,
        dispatch_ready: bool,
    ) -> Result<HostedWorkflowProgress, WorkflowError> {
        if state.status == WorkflowRunStatus::Failed {
            return Ok(self.failed_progress(
                state,
                state
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "workflow failed".to_string()),
            ));
        }
        if state.pending_approval.is_some() || state.status == WorkflowRunStatus::WaitingApproval {
            return Ok(self.awaiting_approval_progress(state, "workflow is waiting for approval"));
        }

        let running_steps = state
            .steps
            .iter()
            .filter(|step| step.status == WorkflowStepStatus::Running)
            .map(|step| step.id.clone())
            .collect::<Vec<_>>();
        if !running_steps.is_empty() {
            return Ok(HostedWorkflowProgress {
                workflow: state.workflow.clone(),
                task: state.task.clone(),
                status: HostedWorkflowProgressStatus::AwaitingOutputs,
                ready_steps: Vec::new(),
                running_steps,
                completed_steps: state.steps_completed,
                total_steps: state.steps_total,
                outputs_available: sorted_keys(&state.outputs),
                pending_approval: state.pending_approval.clone(),
                last_error: state.last_error.clone(),
                message: Some("workflow is waiting for host step outputs".to_string()),
            });
        }

        if state.steps_total == 0
            || state.steps.iter().all(|step| {
                matches!(
                    step.status,
                    WorkflowStepStatus::Completed | WorkflowStepStatus::Skipped
                )
            })
        {
            state.mark_succeeded();
            return Ok(self.succeeded_progress(state, "workflow completed"));
        }

        let validated = crate::dag::DagValidator::new(def).validate()?;
        let step_map: HashMap<String, StepDef> = def
            .steps
            .iter()
            .map(|step| (step.id.clone(), step.clone()))
            .collect();
        let completed = state
            .completed_steps
            .iter()
            .cloned()
            .collect::<HashSet<_>>();

        let mut ready_ids = Vec::new();
        for batch in &validated.batches {
            for step_id in batch {
                let Some(step) = step_map.get(step_id) else {
                    continue;
                };
                let Some(step_state) = state.steps.iter().find(|candidate| candidate.id == step.id)
                else {
                    continue;
                };
                if step_state.status != WorkflowStepStatus::Pending {
                    continue;
                }
                if step_is_ready(step, &state.outputs, &completed) {
                    ready_ids.push(step.id.clone());
                }
            }
            if !ready_ids.is_empty() {
                break;
            }
        }

        if ready_ids.is_empty() {
            state.mark_failed(None, "workflow has no ready or running steps");
            return Ok(
                self.failed_progress(state, "workflow has no ready or running steps".to_string())
            );
        }

        if dispatch_ready {
            for step_id in &ready_ids {
                if let Some(step) = step_map.get(step_id) {
                    state.mark_step_running(step);
                }
            }
        }

        let ready_steps = ready_ids
            .into_iter()
            .filter_map(|step_id| step_map.get(&step_id).cloned())
            .map(|step| self.hosted_step(def, state, &step))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(HostedWorkflowProgress {
            workflow: state.workflow.clone(),
            task: state.task.clone(),
            status: HostedWorkflowProgressStatus::Ready,
            ready_steps,
            running_steps: Vec::new(),
            completed_steps: state.steps_completed,
            total_steps: state.steps_total,
            outputs_available: sorted_keys(&state.outputs),
            pending_approval: state.pending_approval.clone(),
            last_error: state.last_error.clone(),
            message: Some("ready for host execution".to_string()),
        })
    }

    fn hosted_step(
        &self,
        _def: &WorkflowDef,
        state: &WorkflowRunState,
        step: &StepDef,
    ) -> Result<HostedWorkflowStep, WorkflowError> {
        let prompt_template = load_step_template(step)?;
        let prompt =
            render_prompt(&prompt_template, &state.task, &state.outputs).map_err(|error| {
                WorkflowError::TemplateError {
                    id: step.id.clone(),
                    reason: error.to_string(),
                }
            })?;
        let persona = self.persona_provider.get(&step.agent);
        let model_hint = if let Some(model) = &step.model {
            model.clone()
        } else if let Some(persona) = &persona {
            persona.preferred_model.clone()
        } else {
            "host-llm".to_string()
        };

        let mut inputs = HashMap::new();
        for input in step.inputs.iter().flatten() {
            if let Some(value) = state.outputs.get(input) {
                inputs.insert(input.clone(), value.clone());
            }
        }

        Ok(HostedWorkflowStep {
            id: step.id.clone(),
            agent: step.agent.clone(),
            model_hint,
            system_prompt: persona.map(|persona| persona.system_prompt.clone()),
            prompt,
            output_key: step.output.clone(),
            inputs,
            depends_on: step.depends_on.clone().unwrap_or_default(),
            step_type: step.r#type.clone(),
            requires_approval: step.requires_approval.unwrap_or(false),
        })
    }

    fn resolve_submission_content(
        &self,
        state: &mut WorkflowRunState,
        step: &StepDef,
        output: &str,
    ) -> SubmissionResolution {
        if !step.requires_approval.unwrap_or(false) {
            return SubmissionResolution::Content(output.to_string());
        }

        let decision = if let Some(existing) = state.approval_decision(&step.id) {
            existing
        } else if std::env::var("AGENT007_AUTO_APPROVE")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            let decision = ApprovalDecision {
                decision: ApprovalDecisionKind::Approve,
                content: Some(output.to_string()),
            };
            state.record_approval_decision(&step.id, decision.clone());
            decision
        } else {
            state.mark_step_awaiting_approval(step, output);
            return SubmissionResolution::AwaitingApproval(format!(
                "approval required for step '{}'",
                step.id
            ));
        };

        match decision.decision {
            ApprovalDecisionKind::Approve | ApprovalDecisionKind::Edit => {
                SubmissionResolution::Content(
                    decision.content.unwrap_or_else(|| output.to_string()),
                )
            }
            ApprovalDecisionKind::Deny => SubmissionResolution::Failed(
                WorkflowError::ApprovalDenied(step.id.clone()).to_string(),
            ),
        }
    }

    fn complete_step(
        &self,
        def: &WorkflowDef,
        state: &mut WorkflowRunState,
        step: &StepDef,
        content: &str,
    ) {
        if state.status == WorkflowRunStatus::Failed {
            return;
        }

        let step_map: HashMap<String, StepDef> = def
            .steps
            .iter()
            .map(|candidate| (candidate.id.clone(), candidate.clone()))
            .collect();
        let step_dependents = build_step_dependents(def, &step_map);

        if let Some(budget) = &def.budget {
            let token_estimate = estimate_tokens(content);
            let usd_estimate = token_estimate as f64 * 0.000_002;
            let mut used = state.budget_used.clone();
            used.tokens += token_estimate;
            used.estimated_usd += usd_estimate;
            if let Err(error) = check_budget(budget, &used) {
                state.mark_failed(Some(&step.id), error.to_string());
                return;
            }
            state.sync_budget(used);
        }

        let mut outputs = state.outputs.clone();
        if let Some(output_key) = &step.output {
            outputs.insert(output_key.clone(), content.to_string());
        }
        state.mark_step_completed(step, content);
        state.sync_outputs(outputs.clone());

        match step.r#type {
            StepType::Execute | StepType::SubWorkflow => {}
            StepType::Evaluator => {
                let Some(eval) = &step.evaluate else {
                    state.mark_failed(
                        Some(&step.id),
                        WorkflowError::InvalidEvaluator {
                            id: step.id.clone(),
                            reason: "missing evaluate config".to_string(),
                        }
                        .to_string(),
                    );
                    return;
                };

                let passed = if let Some(condition) = &eval.condition {
                    evaluate_condition(condition, &outputs)
                } else if let Some(field) = &eval.decision_field {
                    evaluate_decision_field(content, field)
                } else {
                    true
                };
                let selected_target = if passed {
                    eval.on_pass.clone()
                } else {
                    eval.on_fail.clone()
                };
                state.mark_step_target(&step.id, &selected_target);

                if !passed {
                    let attempt = state.retry_counts.get(&step.id).copied().unwrap_or(0) + 1;
                    let max = eval.max_retries.unwrap_or(3);
                    state.mark_step_retry(&step.id, attempt);
                    if attempt >= max {
                        state.mark_failed(
                            Some(&step.id),
                            WorkflowError::MaxRetriesExceeded {
                                id: step.id.clone(),
                                max,
                            }
                            .to_string(),
                        );
                        return;
                    }

                    let target = eval.on_fail.clone();
                    let mut completed = state
                        .completed_steps
                        .iter()
                        .cloned()
                        .collect::<HashSet<_>>();
                    let mut skipped = state.skipped_steps.iter().cloned().collect::<HashSet<_>>();
                    let mut rewind_outputs = state.outputs.clone();
                    let mut reset_ids = reset_steps_from_target(
                        &target,
                        &step_dependents,
                        &step_map,
                        &mut completed,
                        &mut skipped,
                        state,
                        &mut rewind_outputs,
                    );
                    if !reset_ids.iter().any(|id| id == &step.id) {
                        completed.remove(&step.id);
                        skipped.remove(&step.id);
                        state.reset_step(&step.id);
                        if let Some(output_key) = &step.output {
                            rewind_outputs.remove(output_key);
                        }
                        reset_ids.push(step.id.clone());
                    }
                    reset_ids.sort();
                    reset_ids.dedup();
                    state.completed_steps = completed.into_iter().collect();
                    state.completed_steps.sort();
                    state.skipped_steps = skipped.into_iter().collect();
                    state.skipped_steps.sort();
                    state.steps_completed = state.completed_steps.len();
                    state.sync_outputs(rewind_outputs);
                    return;
                }

                for dependent in step_dependents.get(&step.id).into_iter().flatten() {
                    if dependent != &eval.on_pass {
                        state.mark_step_skipped(dependent);
                    }
                }
            }
            StepType::Router => {
                let Some(routes) = &step.routes else {
                    state.mark_failed(
                        Some(&step.id),
                        WorkflowError::InvalidRouter {
                            id: step.id.clone(),
                            reason: "missing routes config".to_string(),
                        }
                        .to_string(),
                    );
                    return;
                };

                match match_route(content, routes) {
                    Some(selected) => {
                        state.mark_route_selected(&step.id, selected);
                        for route in routes {
                            if route.goto != selected {
                                state.mark_step_skipped(&route.goto);
                            }
                        }
                    }
                    None => {
                        state.mark_failed(
                            Some(&step.id),
                            WorkflowError::NoRouteMatch {
                                id: step.id.clone(),
                                output: content.to_string(),
                            }
                            .to_string(),
                        );
                        return;
                    }
                }
            }
        }

        state.steps_completed = state.completed_steps.len();
        state.status = WorkflowRunStatus::Running;
        state.last_error = None;
    }

    fn awaiting_approval_progress(
        &self,
        state: &WorkflowRunState,
        message: impl Into<String>,
    ) -> HostedWorkflowProgress {
        HostedWorkflowProgress {
            workflow: state.workflow.clone(),
            task: state.task.clone(),
            status: HostedWorkflowProgressStatus::AwaitingApproval,
            ready_steps: Vec::new(),
            running_steps: Vec::new(),
            completed_steps: state.steps_completed,
            total_steps: state.steps_total,
            outputs_available: sorted_keys(&state.outputs),
            pending_approval: state.pending_approval.clone(),
            last_error: state.last_error.clone(),
            message: Some(message.into()),
        }
    }

    fn succeeded_progress(
        &self,
        state: &WorkflowRunState,
        message: impl Into<String>,
    ) -> HostedWorkflowProgress {
        HostedWorkflowProgress {
            workflow: state.workflow.clone(),
            task: state.task.clone(),
            status: HostedWorkflowProgressStatus::Succeeded,
            ready_steps: Vec::new(),
            running_steps: Vec::new(),
            completed_steps: state.steps_completed,
            total_steps: state.steps_total,
            outputs_available: sorted_keys(&state.outputs),
            pending_approval: None,
            last_error: None,
            message: Some(message.into()),
        }
    }

    fn failed_progress(
        &self,
        state: &WorkflowRunState,
        message: impl Into<String>,
    ) -> HostedWorkflowProgress {
        HostedWorkflowProgress {
            workflow: state.workflow.clone(),
            task: state.task.clone(),
            status: HostedWorkflowProgressStatus::Failed,
            ready_steps: Vec::new(),
            running_steps: Vec::new(),
            completed_steps: state.steps_completed,
            total_steps: state.steps_total,
            outputs_available: sorted_keys(&state.outputs),
            pending_approval: state.pending_approval.clone(),
            last_error: state.last_error.clone(),
            message: Some(message.into()),
        }
    }
}

enum SubmissionResolution {
    Content(String),
    AwaitingApproval(String),
    Failed(String),
}

fn load_step_template(step: &StepDef) -> Result<String, WorkflowError> {
    if let Some(prompt) = &step.prompt {
        return Ok(prompt.clone());
    }
    if let Some(skill_trigger) = &step.skill {
        let skills_dir = agent007_core::paths::agent007_home().join("skills");
        let loader = agent007_skills::SkillLoader::new(&skills_dir);
        let skills = loader
            .load_all()
            .map_err(|error| WorkflowError::StepFailed {
                id: step.id.clone(),
                reason: format!("failed to load skills: {error}"),
            })?;
        return skills
            .into_iter()
            .find(|skill| skill.trigger() == skill_trigger)
            .map(|skill| skill.template().to_string())
            .ok_or_else(|| WorkflowError::SkillNotFound(skill_trigger.clone()));
    }
    Err(WorkflowError::StepFailed {
        id: step.id.clone(),
        reason: "step must have either 'prompt' or 'skill'".to_string(),
    })
}

fn sorted_keys(outputs: &HashMap<String, String>) -> Vec<String> {
    let mut keys = outputs.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EvaluateConfig, StepDef, WorkflowDef};
    use agent007_core::persona::NoOpPersonaProvider;

    fn hosted_engine() -> HostedWorkflowEngine {
        HostedWorkflowEngine::new(Arc::new(NoOpPersonaProvider))
    }

    fn single_step_def() -> WorkflowDef {
        WorkflowDef {
            name: "single".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "research".to_string(),
                agent: "Researcher".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("Research {{task}}".to_string()),
                skill: None,
                output: Some("notes".to_string()),
                requires_approval: None,
                r#type: StepType::Execute,
                evaluate: None,
                routes: None,
                workflow: None,
            }],
            budget: None,
        }
    }

    #[test]
    fn hosted_dispatch_and_submit_completes_simple_workflow() {
        let def = single_step_def();
        let mut state = WorkflowRunState::new(&def, "ship feature");
        let engine = hosted_engine();

        let ready = engine.dispatch(&def, &mut state).unwrap();
        assert_eq!(ready.status, HostedWorkflowProgressStatus::Ready);
        assert_eq!(ready.ready_steps.len(), 1);
        assert_eq!(ready.ready_steps[0].id, "research");

        let complete = engine
            .submit_step_output(&def, &mut state, "research", "notes v1")
            .unwrap();
        assert_eq!(complete.status, HostedWorkflowProgressStatus::Succeeded);
        assert_eq!(
            state.outputs.get("notes").map(String::as_str),
            Some("notes v1")
        );
    }

    #[test]
    fn hosted_submit_requires_approval_then_resumes_after_decision() {
        let mut def = single_step_def();
        def.steps[0].requires_approval = Some(true);
        let mut state = WorkflowRunState::new(&def, "ship feature");
        let engine = hosted_engine();

        let _ = engine.dispatch(&def, &mut state).unwrap();
        let waiting = engine
            .submit_step_output(&def, &mut state, "research", "draft plan")
            .unwrap();
        assert_eq!(
            waiting.status,
            HostedWorkflowProgressStatus::AwaitingApproval
        );
        assert!(state.pending_approval.is_some());

        state.record_approval_decision(
            "research",
            ApprovalDecision {
                decision: ApprovalDecisionKind::Edit,
                content: Some("approved plan".to_string()),
            },
        );

        let resumed = engine.dispatch(&def, &mut state).unwrap();
        assert_eq!(resumed.status, HostedWorkflowProgressStatus::Succeeded);
        assert_eq!(
            state.outputs.get("notes").map(String::as_str),
            Some("approved plan")
        );
    }

    #[test]
    fn hosted_evaluator_rewinds_failed_target() {
        let def = WorkflowDef {
            name: "eval".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "impl".to_string(),
                    agent: "Coder".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: None,
                    prompt: Some("Implement {{task}}".to_string()),
                    skill: None,
                    output: Some("code".to_string()),
                    requires_approval: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                    workflow: None,
                },
                StepDef {
                    id: "review".to_string(),
                    agent: "Reviewer".to_string(),
                    model: None,
                    inputs: Some(vec!["code".to_string()]),
                    depends_on: None,
                    prompt: Some("Review {{code}}".to_string()),
                    skill: None,
                    output: Some("verdict".to_string()),
                    requires_approval: None,
                    r#type: StepType::Evaluator,
                    evaluate: Some(EvaluateConfig {
                        condition: None,
                        decision_field: Some("verdict".to_string()),
                        on_pass: "done".to_string(),
                        on_fail: "impl".to_string(),
                        max_retries: Some(3),
                    }),
                    routes: None,
                    workflow: None,
                },
                StepDef {
                    id: "done".to_string(),
                    agent: "Reporter".to_string(),
                    model: None,
                    inputs: Some(vec!["code".to_string()]),
                    depends_on: Some(vec!["review".to_string()]),
                    prompt: Some("Summarize {{code}}".to_string()),
                    skill: None,
                    output: Some("report".to_string()),
                    requires_approval: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                    workflow: None,
                },
            ],
            budget: None,
        };
        let engine = hosted_engine();
        let mut state = WorkflowRunState::new(&def, "ship feature");

        let ready1 = engine.dispatch(&def, &mut state).unwrap();
        assert_eq!(ready1.ready_steps[0].id, "impl");
        let ready2 = engine
            .submit_step_output(&def, &mut state, "impl", "code v1")
            .unwrap();
        assert_eq!(ready2.status, HostedWorkflowProgressStatus::Ready);
        assert_eq!(ready2.ready_steps[0].id, "review");

        let rewound = engine
            .submit_step_output(&def, &mut state, "review", r#"{"verdict":"fail"}"#)
            .unwrap();
        assert_eq!(rewound.status, HostedWorkflowProgressStatus::Ready);
        assert_eq!(rewound.ready_steps[0].id, "impl");
        assert!(!state.outputs.contains_key("code"));
    }
}
