use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use agent007_core::persona::PersonaProvider;
use agent007_core::RunStore;
use agent007_etr::{EtrCallRequest, EtrDispatcher};

use crate::approval::{ApprovalDecision, ApprovalDecisionKind};
use crate::cache::StepCache;
use crate::error::WorkflowError;
use crate::eval_gates::{
    evaluate_workflow_eval_gate, persist_eval_gate_artifacts, EvalGatePolicy,
    WorkflowEvalGateDecisionKind,
};
use crate::recommendations::recommend_route_for_step;
use crate::reliability::{
    apply_degradation, evaluate_budget_decision, evaluate_confidence, evaluate_guardrail,
    BudgetDecision, EscalationDecision, GuardrailDecision, ReliabilityPolicy,
    ReliabilityTransition, ReliabilityTransitionKind,
};
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
    /// Session ID injected so the step agent can self-submit without the orchestrator.
    pub session_id: String,
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
    run_store: Option<Arc<RunStore>>,
    run_id: Option<String>,
}

impl HostedWorkflowEngine {
    pub fn new(persona_provider: Arc<dyn PersonaProvider>) -> Self {
        Self {
            persona_provider,
            run_store: None,
            run_id: None,
        }
    }

    pub fn for_run(&self, run_store: Arc<RunStore>, run_id: impl Into<String>) -> Self {
        Self {
            persona_provider: self.persona_provider.clone(),
            run_store: Some(run_store),
            run_id: Some(run_id.into()),
        }
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
        let reliability_policy = ReliabilityPolicy::from_workflow(def);

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

        let escalation = evaluate_confidence(output, &reliability_policy);
        let force_approval = matches!(escalation, EscalationDecision::RequestApproval { .. });
        if let EscalationDecision::RequestApproval { reason_code } = escalation {
            let transition = ReliabilityTransition::new(
                step.id.clone(),
                ReliabilityTransitionKind::EscalateApproval,
                reason_code,
                Some("confidence policy requested approval".to_string()),
            );
            state.record_reliability_transition(transition);
        }

        let final_content =
            match self.resolve_submission_content(state, step, output, force_approval) {
                SubmissionResolution::AwaitingApproval(message) => {
                    return Ok(self.awaiting_approval_progress(state, message));
                }
                SubmissionResolution::Content(content) => content,
                SubmissionResolution::Failed(message) => {
                    state.mark_failed(Some(step_id), message.clone());
                    return Ok(self.failed_progress(state, message));
                }
            };

        self.complete_step(def, state, step, &final_content, &reliability_policy);
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
                    let reliability_policy = ReliabilityPolicy::from_workflow(def);
                    self.complete_step(def, state, step, &content, &reliability_policy);
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
        let reliability_policy = ReliabilityPolicy::from_workflow(def);
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

        // Build step_map early — needed both for re-delivery of running steps
        // and for the normal ready-step dispatch path below.
        let step_map: HashMap<String, StepDef> = def
            .steps
            .iter()
            .map(|step| (step.id.clone(), step.clone()))
            .collect();

        let running_steps = state
            .steps
            .iter()
            .filter(|step| step.status == WorkflowStepStatus::Running)
            .map(|step| step.id.clone())
            .collect::<Vec<_>>();
        if !running_steps.is_empty() {
            if dispatch_ready {
                // In hosted-MCP mode "Running" means "dispatched to host LLM, awaiting
                // workflow_submit_step". If the host calls workflow_next again before
                // submitting (e.g. it missed the original dispatch response or is
                // polling), we re-deliver the step prompts so it can continue without
                // being permanently stuck. This is an idempotent lease renewal.
                let ready_steps = running_steps
                    .iter()
                    .filter_map(|step_id| step_map.get(step_id).cloned())
                    .map(|step| self.hosted_step(def, state, &step))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(HostedWorkflowProgress {
                    workflow: state.workflow.clone(),
                    task: state.task.clone(),
                    status: HostedWorkflowProgressStatus::Ready,
                    ready_steps,
                    running_steps,
                    completed_steps: state.steps_completed,
                    total_steps: state.steps_total,
                    outputs_available: sorted_keys(&state.outputs),
                    pending_approval: state.pending_approval.clone(),
                    last_error: state.last_error.clone(),
                    message: Some(
                        "re-delivering prompts for in-progress steps (idempotent lease renewal)"
                            .to_string(),
                    ),
                });
            }
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
            self.apply_eval_gate(def, state)?;
            if state.status == WorkflowRunStatus::Failed {
                return Ok(self.failed_progress(
                    state,
                    state
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "eval gate blocked workflow".to_string()),
                ));
            }
            state.mark_succeeded();
            return Ok(self.succeeded_progress(state, "workflow completed"));
        }

        let validated = crate::dag::DagValidator::new(def).validate()?;
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
                    let guardrail_input = self.guardrail_input(def, state, step);
                    match evaluate_guardrail(&step.id, &guardrail_input, &reliability_policy) {
                        GuardrailDecision::Allow { .. } => {
                            ready_ids.push(step.id.clone());
                        }
                        GuardrailDecision::Block {
                            reason_code,
                            category,
                            rationale,
                        } => {
                            let transition = ReliabilityTransition::new(
                                step.id.clone(),
                                ReliabilityTransitionKind::GuardrailBlocked,
                                reason_code,
                                Some(format!("{category}: {rationale}")),
                            );
                            state.record_reliability_transition(transition);
                            state.mark_failed(
                                Some(&step.id),
                                format!("guardrail blocked step '{}'", step.id),
                            );
                            return Ok(self.failed_progress(
                                state,
                                format!("guardrail blocked step '{}'", step.id),
                            ));
                        }
                    }
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
                    // Extract steps are marked running here then immediately completed below.
                    state.mark_step_running(step);
                }
            }
        }

        // Auto-execute Extract steps inline when dispatching (no LLM round-trip needed).
        let mut extract_ran = false;
        let mut non_extract_ids: Vec<String> = Vec::new();
        for step_id in ready_ids {
            let step = match step_map.get(&step_id) {
                Some(s) => s.clone(),
                None => continue,
            };
            if step.r#type != StepType::Extract || !dispatch_ready {
                non_extract_ids.push(step_id);
                continue;
            }
            let Some(extract_cfg) = step.extract.clone() else {
                state.mark_failed(
                    Some(&step.id),
                    "extract step is missing 'extract' config".to_string(),
                );
                return Ok(self.failed_progress(state, "extract step missing config"));
            };

            let workspace_root = std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));

            // Check step cache first when caching is enabled.
            let rendered_input_str = {
                let raw = extract_cfg.input.to_string();
                render_prompt(&raw, &state.task, &state.outputs).unwrap_or(raw)
            };

            let cache_key = if step.cache {
                Some(StepCache::compute_key(&step.id, &rendered_input_str))
            } else {
                None
            };

            if let Some(key) = &cache_key {
                let cache = StepCache::new(&workspace_root);
                if let Some(cached) = cache.get(key) {
                    let mut outputs = state.outputs.clone();
                    if let Some(output_key) = &step.output {
                        outputs.insert(output_key.clone(), cached.clone());
                    }
                    state.mark_step_completed(&step, &cached);
                    state.sync_outputs(outputs);
                    extract_ran = true;
                    continue;
                }
            }

            let input_value: serde_json::Value =
                serde_json::from_str(&rendered_input_str).unwrap_or(extract_cfg.input.clone());

            let dispatcher = EtrDispatcher::new(workspace_root.clone());
            let req = EtrCallRequest {
                tool: extract_cfg.tool.clone(),
                input: input_value,
                compact: extract_cfg.compact,
            };
            let result = dispatcher.call(req);
            let output = if result.status == agent007_etr::EtrStatus::Ok {
                match result.output {
                    serde_json::Value::String(s) => s,
                    v => v.to_string(),
                }
            } else {
                let err = result.error.as_deref().unwrap_or("unknown error");
                format!("[etr error] {err}")
            };

            if let Some(key) = cache_key {
                let cache = StepCache::new(&workspace_root);
                let _ = cache.put(&key, &step.id, &output);
            }

            let mut outputs = state.outputs.clone();
            if let Some(output_key) = &step.output {
                outputs.insert(output_key.clone(), output.clone());
            }
            state.mark_step_completed(&step, &output);
            state.sync_outputs(outputs);
            extract_ran = true;
        }

        // If all ready steps were Extract steps (auto-executed), recurse to get next
        // ready steps rather than returning an empty ready_steps list.
        if extract_ran && non_extract_ids.is_empty() {
            return self.progress(def, state, dispatch_ready);
        }

        let ready_steps = non_extract_ids
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

        let session_id = self.run_id.clone().unwrap_or_default();

        // Append self-submit footer so the step agent can close the loop directly,
        // surviving an orchestrator context compaction.
        let self_submit_footer = if session_id.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n---\n\
                **Self-submit required**: When you finish this step, call `agent007_workflow_submit_step` with:\n\
                - `session`: `{session_id}`\n\
                - `step`: `{step_id}`\n\
                - `output`: your full response\n\n\
                To fetch prior step outputs without bloating the orchestrating context, use:\n\
                `agent007_workflow_get_output(session=\"{session_id}\", key=\"<output_key>\")`\n\n\
                Call `agent007_workflow_heartbeat` every 3-5 minutes while working to report progress and prove liveness:\n\
                `agent007_workflow_heartbeat(session=\"{session_id}\", step=\"{step_id}\", hint=\"<what you are currently doing>\")`\n\
                Skipping heartbeats for >10 min will mark this step as stale in the dashboard.",
                session_id = session_id,
                step_id = step.id,
            )
        };

        Ok(HostedWorkflowStep {
            id: step.id.clone(),
            agent: step.agent.clone(),
            model_hint,
            system_prompt: persona.map(|persona| persona.system_prompt.clone()),
            prompt: format!("{prompt}{self_submit_footer}"),
            output_key: step.output.clone(),
            inputs,
            depends_on: step.depends_on.clone().unwrap_or_default(),
            step_type: step.r#type.clone(),
            requires_approval: step.requires_approval.unwrap_or(false),
            session_id,
        })
    }

    fn guardrail_input(
        &self,
        _def: &WorkflowDef,
        state: &WorkflowRunState,
        step: &StepDef,
    ) -> String {
        let template = match load_step_template(step) {
            Ok(template) => template,
            Err(_) => {
                return step
                    .prompt
                    .clone()
                    .or_else(|| step.skill.clone())
                    .unwrap_or_default();
            }
        };

        render_prompt(&template, &state.task, &state.outputs).unwrap_or(template)
    }

    fn resolve_submission_content(
        &self,
        state: &mut WorkflowRunState,
        step: &StepDef,
        output: &str,
        force_approval: bool,
    ) -> SubmissionResolution {
        if !step.requires_approval.unwrap_or(false) && !force_approval {
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
                "approval required for step '{}' ({})",
                step.id,
                if force_approval {
                    "confidence-escalation"
                } else {
                    "step-config"
                }
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
        reliability_policy: &ReliabilityPolicy,
    ) {
        if state.status == WorkflowRunStatus::Failed {
            return;
        }
        let mut final_content = content.to_string();

        let step_map: HashMap<String, StepDef> = def
            .steps
            .iter()
            .map(|candidate| (candidate.id.clone(), candidate.clone()))
            .collect();
        let step_dependents = build_step_dependents(def, &step_map);

        if let Some(budget) = &def.budget {
            let mut used = state.budget_used.clone();
            let token_estimate = estimate_tokens(&final_content);
            let usd_estimate = token_estimate as f64 * 0.000_002;
            match evaluate_budget_decision(
                budget,
                &used,
                token_estimate,
                usd_estimate,
                state.degradation_count,
                reliability_policy,
            ) {
                BudgetDecision::Continue { .. } => {
                    used.tokens += token_estimate;
                    used.estimated_usd += usd_estimate;
                    if let Err(error) = check_budget(budget, &used) {
                        state.record_reliability_transition(ReliabilityTransition::new(
                            step.id.clone(),
                            ReliabilityTransitionKind::Abort,
                            "budget-limit-exceeded",
                            Some(error.to_string()),
                        ));
                        state.mark_failed(Some(&step.id), error.to_string());
                        return;
                    }
                }
                BudgetDecision::Degrade {
                    reason_code,
                    target_chars,
                } => {
                    let degraded = apply_degradation(&final_content, target_chars);
                    let degraded_tokens = estimate_tokens(&degraded);
                    let degraded_usd = degraded_tokens as f64 * 0.000_002;
                    let mut projected = used.clone();
                    projected.tokens += degraded_tokens;
                    projected.estimated_usd += degraded_usd;
                    if let Err(error) = check_budget(budget, &projected) {
                        state.record_reliability_transition(ReliabilityTransition::new(
                            step.id.clone(),
                            ReliabilityTransitionKind::Abort,
                            "budget-degrade-failed",
                            Some(error.to_string()),
                        ));
                        state.mark_failed(Some(&step.id), error.to_string());
                        return;
                    }
                    final_content = degraded;
                    used = projected;
                    state.sync_degradation_count(state.degradation_count.saturating_add(1));
                    state.record_reliability_transition(ReliabilityTransition::new(
                        step.id.clone(),
                        ReliabilityTransitionKind::Degrade,
                        reason_code,
                        Some(format!(
                            "output truncated to {} chars to remain within budget",
                            target_chars
                        )),
                    ));
                }
                BudgetDecision::Abort { reason_code } => {
                    state.record_reliability_transition(ReliabilityTransition::new(
                        step.id.clone(),
                        ReliabilityTransitionKind::Abort,
                        reason_code.clone(),
                        Some("budget governor aborted execution".to_string()),
                    ));
                    state.mark_failed(Some(&step.id), reason_code);
                    return;
                }
            }
            state.sync_budget(used);
        }

        let mut outputs = state.outputs.clone();
        if let Some(output_key) = &step.output {
            outputs.insert(output_key.clone(), final_content.clone());
        }
        state.mark_step_completed(step, &final_content);
        state.sync_outputs(outputs.clone());

        match step.r#type {
            StepType::Execute | StepType::SubWorkflow | StepType::Extract => {}
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
                    evaluate_decision_field(&final_content, field)
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
                    state.record_reliability_transition(ReliabilityTransition::new(
                        step.id.clone(),
                        ReliabilityTransitionKind::Retry,
                        "evaluator-failed-retry",
                        Some(format!("attempt {attempt} of {max}")),
                    ));
                    state.mark_step_retry(&step.id, attempt);
                    if attempt >= max {
                        state.record_reliability_transition(ReliabilityTransition::new(
                            step.id.clone(),
                            ReliabilityTransitionKind::Abort,
                            "max-retries-exceeded",
                            Some(format!("attempt {attempt} reached max {max}")),
                        ));
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

                match match_route(&final_content, routes) {
                    Some(selected) => {
                        if let Some(store) = &self.run_store {
                            let candidates = routes
                                .iter()
                                .map(|route| route.goto.clone())
                                .collect::<Vec<_>>();
                            let recommendation = recommend_route_for_step(
                                store,
                                &def.name,
                                &step.id,
                                selected,
                                &candidates,
                                self.run_id.as_deref(),
                            );
                            state.record_routing_recommendation(recommendation);
                        }
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
                                output: final_content.to_string(),
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
        state.record_reliability_transition(ReliabilityTransition::new(
            step.id.clone(),
            ReliabilityTransitionKind::Continue,
            "step-completed",
            None,
        ));
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

    fn apply_eval_gate(
        &self,
        def: &WorkflowDef,
        state: &mut WorkflowRunState,
    ) -> Result<(), WorkflowError> {
        if state.eval_gate_decision.is_some() {
            return Ok(());
        }
        let Some(store) = &self.run_store else {
            return Ok(());
        };
        let Some(run_id) = &self.run_id else {
            return Ok(());
        };

        let policy = EvalGatePolicy::from_workflow(def);
        let Some(decision) =
            evaluate_workflow_eval_gate(store, run_id, &def.name, &state.budget_used, &policy)?
        else {
            return Ok(());
        };

        state.set_eval_gate_decision(decision.clone());
        let _ = persist_eval_gate_artifacts(store, run_id, &decision);

        if matches!(decision.decision, WorkflowEvalGateDecisionKind::Block) {
            state.mark_failed(
                None,
                format!(
                    "eval gate blocked workflow '{}': {}",
                    def.name, decision.message
                ),
            );
        }
        Ok(())
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
    use crate::types::{
        BudgetConfig, EvalGateConfig, EvalGateMode, EvalGateThresholdConfig, EvaluateConfig,
        StepDef, WorkflowDef,
    };
    use agent007_core::persona::NoOpPersonaProvider;
    use agent007_core::{RunScorecard, RunStatus, RunStore};
    use chrono::Utc;

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
                ..Default::default()
            }],
            budget: None,
            reliability: None,
            eval_gate: None,
        }
    }

    fn release_gated_def(mode: EvalGateMode) -> WorkflowDef {
        let mut def = single_step_def();
        def.budget = Some(BudgetConfig {
            max_tokens_per_session: Some(10_000),
            max_usd_per_task: Some(1.0),
            alert_at_percent: None,
            on_exceed: Some("alert-only".to_string()),
        });
        def.eval_gate = Some(EvalGateConfig {
            enabled: Some(true),
            release_class: Some(true),
            mode: Some(mode),
            baseline_window: Some(5),
            min_baseline_runs: Some(3),
            thresholds: Some(EvalGateThresholdConfig {
                max_quality_score_drop: Some(100.0),
                max_cost_usd_increase: Some(0.0),
                max_latency_ms_increase: Some(60_000.0),
                max_retry_increase: Some(10.0),
            }),
        });
        def
    }

    fn seed_baseline_scorecard(store: &RunStore, workflow: &str, cost_usd: f64) {
        let run = store
            .create_run("workflow-test", "baseline", "standalone", Some("mock"))
            .unwrap();
        store
            .write_json_artifact(
                &run.id,
                "workflow-request.json",
                &serde_json::json!({
                    "workflow": workflow,
                    "task": "baseline"
                }),
            )
            .unwrap();
        let finished = store.finish_run(&run.id, true, "baseline ok").unwrap();
        let scorecard = RunScorecard {
            schema_version: 1,
            run_id: run.id.clone(),
            kind: "workflow-test".to_string(),
            workflow: Some(workflow.to_string()),
            mode: finished.mode.clone(),
            provider: finished.provider.clone(),
            status: RunStatus::Succeeded,
            completed: true,
            success: true,
            started_at: finished.started_at,
            finished_at: finished.finished_at,
            duration_ms: Some(500),
            tokens: 0,
            requests: 1,
            estimated_usd: cost_usd,
            retry_count: 0,
            tool_calls: 1,
            tool_errors: 0,
            quality_score: 99.0,
            updated_at: Utc::now(),
        };
        store
            .write_json_artifact(&run.id, "run-scorecard.json", &scorecard)
            .unwrap();
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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
                },
            ],
            budget: None,
            reliability: None,
            eval_gate: None,
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

    #[test]
    fn hosted_router_records_shadow_routing_recommendation() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RunStore::new(dir.path()));
        for (route, quality) in [
            ("ui-work", 40.0),
            ("ui-work", 41.0),
            ("api-work", 93.0),
            ("api-work", 94.0),
        ] {
            let run = store
                .create_run("workflow-test", "baseline", "standalone", Some("mock"))
                .unwrap();
            store
                .write_json_artifact(
                    &run.id,
                    "workflow-request.json",
                    &serde_json::json!({
                        "workflow": "router-hosted",
                        "task": "baseline"
                    }),
                )
                .unwrap();
            store
                .write_json_artifact(
                    &run.id,
                    "workflow-state.json",
                    &serde_json::json!({
                        "workflow": "router-hosted",
                        "task": "baseline",
                        "status": "succeeded",
                        "steps_total": 1,
                        "steps_completed": 1,
                        "completed_steps": ["classify"],
                        "skipped_steps": [],
                        "retry_counts": {},
                        "recovery_retry_counts": {},
                        "outputs": {},
                        "budget_used": { "tokens": 0, "estimated_usd": 0.0 },
                        "degradation_count": 0,
                        "reliability_transitions": [],
                        "reliability_events": [],
                        "eval_gate_decision": null,
                        "routing_recommendations": [],
                        "steps": [{
                            "id": "classify",
                            "agent": "Router",
                            "status": "completed",
                            "attempts": 1,
                            "output_key": "classification",
                            "output_preview": route,
                            "selected_route": route,
                            "selected_target": null,
                            "error": null
                        }],
                        "pending_approval": null,
                        "approval_decisions": {},
                        "last_error": null
                    }),
                )
                .unwrap();
            let finished = store.finish_run(&run.id, true, "baseline ok").unwrap();
            let retry_count = ((100.0_f64 - quality).max(0.0) / 4.0).round() as u32;
            store
                .write_json_artifact(
                    &run.id,
                    "run-scorecard.json",
                    &RunScorecard {
                        schema_version: 1,
                        run_id: run.id.clone(),
                        kind: "workflow-test".to_string(),
                        workflow: Some("router-hosted".to_string()),
                        mode: finished.mode.clone(),
                        provider: finished.provider.clone(),
                        status: RunStatus::Succeeded,
                        completed: true,
                        success: true,
                        started_at: finished.started_at,
                        finished_at: finished.finished_at,
                        duration_ms: Some(100),
                        tokens: 0,
                        requests: 1,
                        estimated_usd: 0.0,
                        retry_count,
                        tool_calls: 0,
                        tool_errors: 0,
                        quality_score: quality,
                        updated_at: Utc::now(),
                    },
                )
                .unwrap();
        }

        let run = store
            .create_run("workflow-test", "ship feature", "standalone", Some("mock"))
            .unwrap();
        let engine = hosted_engine().for_run(store.clone(), run.id.clone());
        let def = WorkflowDef {
            name: "router-hosted".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "classify".to_string(),
                    agent: "Router".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: None,
                    prompt: Some("classify {{task}}".to_string()),
                    skill: None,
                    output: Some("classification".to_string()),
                    requires_approval: None,
                    r#type: StepType::Router,
                    evaluate: None,
                    routes: Some(vec![
                        crate::types::RouteConfig {
                            when: Some("ui-work".to_string()),
                            goto: "ui-work".to_string(),
                            default: false,
                        },
                        crate::types::RouteConfig {
                            when: Some("api-work".to_string()),
                            goto: "api-work".to_string(),
                            default: false,
                        },
                    ]),
                    workflow: None,
                    ..Default::default()
                },
                StepDef {
                    id: "ui-work".to_string(),
                    agent: "UI".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: Some(vec!["classify".to_string()]),
                    prompt: Some("ui {{task}}".to_string()),
                    skill: None,
                    output: Some("ui_result".to_string()),
                    requires_approval: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                    workflow: None,
                    ..Default::default()
                },
                StepDef {
                    id: "api-work".to_string(),
                    agent: "API".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: Some(vec!["classify".to_string()]),
                    prompt: Some("api {{task}}".to_string()),
                    skill: None,
                    output: Some("api_result".to_string()),
                    requires_approval: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                    workflow: None,
                    ..Default::default()
                },
            ],
            budget: None,
            reliability: None,
            eval_gate: None,
        };

        let mut state = WorkflowRunState::new(&def, "ship feature");
        let ready = engine.dispatch(&def, &mut state).unwrap();
        assert_eq!(ready.status, HostedWorkflowProgressStatus::Ready);

        let done = engine
            .submit_step_output(&def, &mut state, "classify", "ui-work")
            .unwrap();
        assert_eq!(done.status, HostedWorkflowProgressStatus::Ready);
        assert_eq!(state.routing_recommendations.len(), 1);
        let recommendation = &state.routing_recommendations[0];
        assert_eq!(recommendation.step_id, "classify");
        assert_eq!(recommendation.current_route, "ui-work");
        assert_eq!(recommendation.recommended_route, "api-work");
        assert!(!recommendation.fallback_used);
    }

    #[test]
    fn hosted_confidence_escalation_requests_approval() {
        let _guard = crate::reliability::test_env_lock();
        std::env::set_var("AGENT007_RELIABILITY_ENABLED", "1");
        std::env::set_var("AGENT007_RELIABILITY_CONFIDENCE_ESCALATION", "1");
        std::env::set_var("AGENT007_AUTO_APPROVE", "0");

        let def = single_step_def();
        let engine = hosted_engine();
        let mut state = WorkflowRunState::new(&def, "ship feature");

        let ready = engine.dispatch(&def, &mut state).unwrap();
        assert_eq!(ready.status, HostedWorkflowProgressStatus::Ready);

        let waiting = engine
            .submit_step_output(&def, &mut state, "research", "draft\nconfidence: low")
            .unwrap();
        assert_eq!(
            waiting.status,
            HostedWorkflowProgressStatus::AwaitingApproval
        );
        assert!(state.pending_approval.is_some());
        assert!(!state.reliability_transitions.is_empty());

        std::env::remove_var("AGENT007_RELIABILITY_ENABLED");
        std::env::remove_var("AGENT007_RELIABILITY_CONFIDENCE_ESCALATION");
        std::env::remove_var("AGENT007_AUTO_APPROVE");
    }

    #[test]
    fn hosted_eval_gate_fail_closed_blocks_regressed_release_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RunStore::new(dir.path()));
        for _ in 0..3 {
            seed_baseline_scorecard(&store, "single", 0.0);
        }
        let run = store
            .create_run("workflow-test", "ship feature", "standalone", Some("mock"))
            .unwrap();
        let engine = hosted_engine().for_run(store.clone(), run.id.clone());
        let def = release_gated_def(EvalGateMode::FailClosed);
        let mut state = WorkflowRunState::new(&def, "ship feature");

        let ready = engine.dispatch(&def, &mut state).unwrap();
        assert_eq!(ready.status, HostedWorkflowProgressStatus::Ready);

        let blocked = engine
            .submit_step_output(&def, &mut state, "research", "current release output")
            .unwrap();
        assert_eq!(blocked.status, HostedWorkflowProgressStatus::Failed);
        assert_eq!(state.status, WorkflowRunStatus::Failed);
        assert_eq!(
            state
                .eval_gate_decision
                .as_ref()
                .map(|decision| decision.decision.clone()),
            Some(crate::eval_gates::WorkflowEvalGateDecisionKind::Block)
        );
        assert!(store
            .read_json_artifact_optional::<serde_json::Value>(&run.id, "eval-gate-decision.json")
            .unwrap()
            .is_some());
    }

    #[test]
    fn hosted_budget_governor_degrades_output() {
        let _guard = crate::reliability::test_env_lock();
        std::env::set_var("AGENT007_RELIABILITY_ENABLED", "1");
        std::env::set_var("AGENT007_RELIABILITY_BUDGET_GOVERNOR", "1");
        std::env::set_var("AGENT007_RELIABILITY_DEGRADE_OUTPUT_CHARS", "4");
        std::env::set_var("AGENT007_RELIABILITY_MAX_DEGRADATIONS", "1");

        let mut def = single_step_def();
        def.budget = Some(crate::types::BudgetConfig {
            max_tokens_per_session: Some(4),
            max_usd_per_task: None,
            alert_at_percent: None,
            on_exceed: Some("stop".to_string()),
        });

        let engine = hosted_engine();
        let mut state = WorkflowRunState::new(&def, "ship feature");
        let _ = engine.dispatch(&def, &mut state).unwrap();

        let done = engine
            .submit_step_output(&def, &mut state, "research", "abcdefghijklmnopqrstuvwxyz")
            .unwrap();
        assert_eq!(done.status, HostedWorkflowProgressStatus::Succeeded);
        assert!(state
            .outputs
            .get("notes")
            .map(|value| value.contains("[degraded]"))
            .unwrap_or(false));

        std::env::remove_var("AGENT007_RELIABILITY_ENABLED");
        std::env::remove_var("AGENT007_RELIABILITY_BUDGET_GOVERNOR");
        std::env::remove_var("AGENT007_RELIABILITY_DEGRADE_OUTPUT_CHARS");
        std::env::remove_var("AGENT007_RELIABILITY_MAX_DEGRADATIONS");
    }

    // Regression test for the hosted-MCP "stuck Running steps" bug.
    // Scenario: workflow_start dispatches the first step (marking it Running) but the
    // host LLM then calls workflow_next again before submitting — e.g. because it polled
    // for status or the original dispatch response was lost. Previously workflow_next
    // returned ready_steps: [] (AwaitingOutputs), permanently stranding the workflow.
    // After the fix, workflow_next re-delivers the running step prompts (idempotent
    // lease renewal) so the host can recover without any workaround.
    #[test]
    fn workflow_next_redelivers_running_steps_when_host_polls_again() {
        let def = single_step_def();
        let engine = hosted_engine();
        let mut state = WorkflowRunState::new(&def, "ship feature");

        // Simulate workflow_start: dispatch marks the step Running and returns it.
        let first_dispatch = engine.dispatch(&def, &mut state).unwrap();
        assert_eq!(first_dispatch.status, HostedWorkflowProgressStatus::Ready);
        assert_eq!(first_dispatch.ready_steps.len(), 1);
        assert_eq!(first_dispatch.ready_steps[0].id, "research");

        // Step is now Running. Host calls workflow_next again without having submitted.
        // This should re-deliver the prompt (not return empty ready_steps).
        let redelivered = engine.dispatch(&def, &mut state).unwrap();
        assert_eq!(
            redelivered.status,
            HostedWorkflowProgressStatus::Ready,
            "workflow_next must return Ready (not AwaitingOutputs) when steps are Running"
        );
        assert_eq!(
            redelivered.ready_steps.len(),
            1,
            "workflow_next must re-deliver the Running step prompt"
        );
        assert_eq!(redelivered.ready_steps[0].id, "research");
        assert!(
            redelivered
                .message
                .as_deref()
                .unwrap_or("")
                .contains("re-delivering"),
            "message should indicate this is a re-delivery"
        );

        // status() (dispatch_ready=false) should still report AwaitingOutputs.
        let status = engine.status(&def, &mut state).unwrap();
        assert_eq!(status.status, HostedWorkflowProgressStatus::AwaitingOutputs);
        assert_eq!(status.ready_steps.len(), 0);

        // Normal completion path should still work after re-delivery.
        let done = engine
            .submit_step_output(&def, &mut state, "research", "final answer")
            .unwrap();
        assert_eq!(done.status, HostedWorkflowProgressStatus::Succeeded);
    }
}
