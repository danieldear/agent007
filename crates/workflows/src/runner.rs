use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::sync::Arc;
use tokio::sync::Mutex;

use agent007_core::dispatcher::Dispatcher;
use agent007_core::events::AgentEvent;
use agent007_core::persona::PersonaProvider;
use agent007_core::types::PromptRef;
use agent007_core::RunStore;
use agent007_models::{CompletionRequest, Message, ModelProvider, ModelRouter, Role};

use crate::approval::{ApprovalDecision, ApprovalDecisionKind, ApprovalGate};
use crate::dag::DagValidator;
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
use crate::state::WorkflowRunState;
use crate::types::{BudgetUsed, StepType, WorkflowDef, WorkflowResult};

pub struct WorkflowRunner {
    pub persona_provider: Arc<dyn PersonaProvider>,
    pub model_router: Arc<ModelRouter>,
    pub dispatcher: Arc<dyn Dispatcher>,
    pub run_store: Option<Arc<RunStore>>,
    pub run_id: Option<String>,
    pub initial_state: Option<WorkflowRunState>,
    /// Optional skill content provider for multi-agent steps.
    /// When `Some`, it is shared across all MultiAgent step executions in this
    /// run — built once at runner construction, avoiding per-step disk reads.
    /// When `None`, a `NoOpSkillContentProvider` fallback is used with a
    /// warning logged.
    pub skill_content_provider: Option<Arc<dyn agent007_skills::SkillContentProvider>>,
}

impl WorkflowRunner {
    pub fn new(
        persona_provider: Arc<dyn PersonaProvider>,
        model_router: Arc<ModelRouter>,
        dispatcher: Arc<dyn Dispatcher>,
    ) -> Self {
        Self {
            persona_provider,
            model_router,
            dispatcher,
            run_store: None,
            run_id: None,
            initial_state: None,
            skill_content_provider: None,
        }
    }

    /// Attach a pre-built `SkillContentProvider` to this runner.
    ///
    /// Call this after `new()` when the caller has already loaded skills from
    /// disk.  Avoids the per-step filesystem reload that happens when the field
    /// is absent.
    pub fn with_skill_provider(
        mut self,
        provider: Arc<dyn agent007_skills::SkillContentProvider>,
    ) -> Self {
        self.skill_content_provider = Some(provider);
        self
    }

    pub fn for_run(&self, run_store: Arc<RunStore>, run_id: impl Into<String>) -> Self {
        Self {
            persona_provider: self.persona_provider.clone(),
            model_router: self.model_router.clone(),
            dispatcher: self.dispatcher.clone(),
            run_store: Some(run_store),
            run_id: Some(run_id.into()),
            skill_content_provider: self.skill_content_provider.clone(),
            initial_state: None,
        }
    }

    pub fn resume_from(
        &self,
        run_store: Arc<RunStore>,
        run_id: impl Into<String>,
        state: WorkflowRunState,
    ) -> Self {
        Self {
            persona_provider: self.persona_provider.clone(),
            model_router: self.model_router.clone(),
            dispatcher: self.dispatcher.clone(),
            run_store: Some(run_store),
            run_id: Some(run_id.into()),
            initial_state: Some(state),
            skill_content_provider: self.skill_content_provider.clone(),
        }
    }

    /// Validate the DAG and return topological batches plus evaluator/router metadata. Public so
    /// the CLI `validate` command can call it without running steps.
    pub fn validate(&self, def: &WorkflowDef) -> Result<crate::dag::ValidatedDag, WorkflowError> {
        DagValidator::new(def).validate()
    }

    /// Run the full workflow. `task_input` fills the `{{task}}` Tera variable.
    pub async fn run(
        &self,
        def: &WorkflowDef,
        task_input: &str,
    ) -> Result<WorkflowResult, WorkflowError> {
        let mut state = self
            .initial_state
            .clone()
            .unwrap_or_else(|| WorkflowRunState::new(def, task_input));
        self.persist_workflow_artifacts(&state);
        if self.initial_state.is_some() {
            self.trace_note(
                "workflow-resume",
                serde_json::json!({
                    "workflow": def.name,
                    "task": task_input,
                    "steps_total": def.steps.len(),
                    "completed_steps": state.completed_steps.clone(),
                    "skipped_steps": state.skipped_steps.clone(),
                }),
            );
        } else {
            self.trace_note(
                "workflow-start",
                serde_json::json!({
                    "workflow": def.name,
                    "task": task_input,
                    "steps_total": def.steps.len(),
                }),
            );
        }

        let validated_dag = match self.validate(def) {
            Ok(validated) => validated,
            Err(error) => return fail_workflow(self, &mut state, None, error),
        };
        let steps_total = def.steps.len();

        let mut skipped_steps: HashSet<String> = state.skipped_steps.iter().cloned().collect();
        let mut completed_steps: HashSet<String> = state.completed_steps.iter().cloned().collect();
        let mut evaluator_retry_counts: HashMap<String, u32> = state.retry_counts.clone();
        let mut recovery_retry_counts: HashMap<String, u32> = state.recovery_retry_counts.clone();

        // Shared output artifact store, protected by a Mutex for concurrent batch steps.
        let outputs: Arc<Mutex<HashMap<String, String>>> =
            Arc::new(Mutex::new(state.outputs.clone()));
        let budget_used: Arc<Mutex<BudgetUsed>> = Arc::new(Mutex::new(state.budget_used.clone()));
        let reliability_policy = ReliabilityPolicy::from_workflow(def);
        let mut degradation_count = state.degradation_count;
        let mut steps_completed = state.steps_completed;

        // Build a lookup: step_id → StepDef
        let step_map: HashMap<String, crate::types::StepDef> = def
            .steps
            .iter()
            .map(|s| (s.id.clone(), s.clone()))
            .collect();
        let step_batches = build_step_batch_map(&validated_dag.batches);
        let step_dependents = build_step_dependents(def, &step_map);

        let mut batch_index = 0_usize;
        'workflow_batches: while batch_index < validated_dag.batches.len() {
            let batch = &validated_dag.batches[batch_index];
            self.trace_note(
                "workflow-batch-start",
                serde_json::json!({
                    "workflow": def.name,
                    "batch_index": batch_index,
                    "batch": batch,
                }),
            );
            let mut pending: Vec<String> = batch
                .iter()
                .filter(|id| !skipped_steps.contains(*id) && !completed_steps.contains(*id))
                .cloned()
                .collect();

            while !pending.is_empty() {
                let snapshot = outputs.lock().await.clone();
                let ready: Vec<String> = pending
                    .iter()
                    .filter(|id| {
                        let step = step_map.get(*id).unwrap();
                        step_is_ready(step, &snapshot, &completed_steps)
                    })
                    .cloned()
                    .collect();

                if ready.is_empty() {
                    return fail_workflow(
                        self,
                        &mut state,
                        Some(pending[0].clone()),
                        WorkflowError::StepFailed {
                        id: pending[0].clone(),
                        reason: "depends_on or inputs not satisfied within batch (scheduling deadlock)"
                            .to_string(),
                        },
                    );
                }

                pending.retain(|id| !ready.contains(id));

                let mut rewind_to_batch = None;
                let mut rewind_target = None;
                let mut step_futures = Vec::new();
                for step_id in &ready {
                    let step = step_map.get(step_id).unwrap().clone();
                    let guardrail_input = step
                        .prompt
                        .as_deref()
                        .and_then(|template| render_prompt(template, task_input, &snapshot).ok())
                        .or_else(|| step.prompt.clone())
                        .or_else(|| step.skill.clone())
                        .unwrap_or_default();
                    match evaluate_guardrail(&step.id, &guardrail_input, &reliability_policy) {
                        GuardrailDecision::Allow { .. } => {}
                        GuardrailDecision::Block {
                            reason_code,
                            category,
                            rationale,
                        } => {
                            let transition = ReliabilityTransition::new(
                                step.id.clone(),
                                ReliabilityTransitionKind::GuardrailBlocked,
                                reason_code.clone(),
                                Some(rationale.clone()),
                            );
                            state.record_reliability_transition(transition.clone());
                            state.sync_degradation_count(degradation_count);
                            self.trace_note(
                                "workflow-guardrail-decision",
                                serde_json::json!({
                                    "workflow": def.name,
                                    "step_id": step.id,
                                    "decision": "block",
                                    "reason_code": reason_code,
                                    "category": category,
                                    "rationale": rationale,
                                }),
                            );
                            self.trace_note(
                                "workflow-reliability-transition",
                                serde_json::json!({
                                    "workflow": def.name,
                                    "transition": transition,
                                }),
                            );
                            return fail_workflow(
                                self,
                                &mut state,
                                Some(step.id.clone()),
                                WorkflowError::StepFailed {
                                    id: step.id.clone(),
                                    reason: format!("guardrail blocked step '{}'", step.id),
                                },
                            );
                        }
                    }
                    let pending_approval_content = state
                        .pending_approval
                        .as_ref()
                        .filter(|pending| pending.step_id == step.id)
                        .map(|pending| pending.content.clone());
                    state.mark_step_running(&step);
                    state.retry_counts = evaluator_retry_counts.clone();
                    state.recovery_retry_counts = recovery_retry_counts.clone();
                    self.persist_workflow_artifacts(&state);
                    self.trace_note(
                        "workflow-step-dispatched",
                        serde_json::json!({
                            "workflow": def.name,
                            "step_id": step.id,
                            "agent": step.agent,
                            "model": step.model,
                            "type": step.r#type,
                            "output": step.output,
                        }),
                    );
                    let task_str = task_input.to_string();
                    let ctx_outputs = snapshot.clone();
                    let router = self.model_router.clone();
                    let persona_provider = self.persona_provider.clone();
                    let sub_dispatcher = self.dispatcher.clone();

                    if let Some(existing_content) = pending_approval_content {
                        step_futures.push(tokio::spawn(async move {
                            Ok::<(crate::types::StepDef, String, String, usize), WorkflowError>((
                                step,
                                existing_content,
                                String::new(),
                                0,
                            ))
                        }));
                        continue;
                    }

                    // Sub-workflow steps run inline (not in tokio::spawn) to avoid Send bounds
                    if step.r#type == StepType::SubWorkflow {
                        let sub_result: Result<
                            (crate::types::StepDef, String, String, usize),
                            WorkflowError,
                        > = async {
                            let wf_name =
                                step.workflow
                                    .clone()
                                    .ok_or_else(|| WorkflowError::StepFailed {
                                        id: step.id.clone(),
                                        reason: "sub-workflow step missing 'workflow' field"
                                            .to_string(),
                                    })?;
                            let wf_path = agent007_core::paths::agent007_home()
                                .join("workflows")
                                .join(format!("{wf_name}.toml"));
                            let toml_str = std::fs::read_to_string(&wf_path).map_err(|e| {
                                WorkflowError::StepFailed {
                                    id: step.id.clone(),
                                    reason: format!("failed to read sub-workflow '{wf_name}': {e}"),
                                }
                            })?;
                            let sub_def: crate::types::WorkflowDef = toml::from_str(&toml_str)
                                .map_err(|e| WorkflowError::StepFailed {
                                    id: step.id.clone(),
                                    reason: format!(
                                        "failed to parse sub-workflow '{wf_name}': {e}"
                                    ),
                                })?;
                            let sub_runner = WorkflowRunner::new(
                                persona_provider.clone(),
                                router.clone(),
                                sub_dispatcher.clone(),
                            );
                            let result = Box::pin(sub_runner.run(&sub_def, &task_str))
                                .await
                                .map_err(|e| WorkflowError::StepFailed {
                                    id: step.id.clone(),
                                    reason: format!("sub-workflow '{wf_name}' failed: {e}"),
                                })?;
                            let content = result
                                .outputs
                                .iter()
                                .map(|(k, v)| format!("{k}: {v}"))
                                .collect::<Vec<_>>()
                                .join("\n\n");
                            let tokens = result.budget_used.tokens as usize;
                            Ok((step, content, format!("sub-workflow/{wf_name}"), tokens))
                        }
                        .await;
                        step_futures.push(tokio::spawn(async move { sub_result }));
                        continue;
                    }

                    // MultiAgent steps build a SubOrchestrator from the step's persona and
                    // worker list, inject skill domain knowledge, and run inline (not spawned)
                    // to avoid requiring the orchestrator and its deps to be Send.
                    if step.r#type == StepType::MultiAgent {
                        // Capture the skill provider from the runner field, or fall back to a
                        // lazy disk load (and warn so the caller knows to wire it up properly).
                        let skill_provider_for_step: Arc<
                            dyn agent007_skills::SkillContentProvider,
                        > = {
                            if let Some(ref sp) = self.skill_content_provider {
                                Arc::clone(sp)
                            } else {
                                tracing::warn!(
                                    step_id = %step.id,
                                    "WorkflowRunner.skill_content_provider not set; loading skills \
                                     from disk for this step. Use with_skill_provider() at runner \
                                     construction to avoid per-step disk reads."
                                );
                                let skills_dir =
                                    agent007_core::paths::agent007_home().join("skills");
                                match agent007_skills::SkillLoader::new(&skills_dir).load_all() {
                                    Ok(skills) => {
                                        Arc::new(agent007_skills::SkillIndex::from_skills(skills))
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %e,
                                            "failed to load skills; multi-agent step will run \
                                             without skill injection"
                                        );
                                        Arc::new(agent007_skills::NoOpSkillContentProvider)
                                    }
                                }
                            }
                        };

                        let multi_result: Result<
                            (crate::types::StepDef, String, String, usize),
                            WorkflowError,
                        > = async {
                            // Resolve the orchestrator persona.
                            let persona_name = step.agent.clone();
                            let persona_spec =
                                persona_provider.get(&persona_name).ok_or_else(|| {
                                    WorkflowError::StepFailed {
                                        id: step.id.clone(),
                                        reason: format!(
                                        "persona '{persona_name}' not found for multi-agent step"
                                    ),
                                    }
                                })?;
                            if !matches!(
                                persona_spec.agent_type.as_deref(),
                                Some(kind) if kind.eq_ignore_ascii_case("orchestrator")
                            ) {
                                return Err(WorkflowError::StepFailed {
                                    id: step.id.clone(),
                                    reason: format!(
                                        "persona '{persona_name}' must have agent_type = 'orchestrator' for multi-agent step"
                                    ),
                                });
                            }

                            // Map WorkerConfig → WorkerSpec to avoid a circular dep between
                            // workflows ← custom-agents.
                            let worker_specs: Vec<agent007_custom_agents::WorkerSpec> = step
                                .workers
                                .as_ref()
                                .map(|ws| {
                                    ws.iter()
                                        .map(|wc| agent007_custom_agents::WorkerSpec {
                                            name: wc.persona.clone(),
                                            skills: wc.skills.clone(),
                                            sequential: wc.run
                                                == crate::types::WorkerRunMode::Sequential,
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            for worker in &worker_specs {
                                let worker_persona =
                                    persona_provider.get(&worker.name).ok_or_else(|| {
                                        WorkflowError::StepFailed {
                                            id: step.id.clone(),
                                            reason: format!(
                                                "worker persona '{}' not found for multi-agent step",
                                                worker.name
                                            ),
                                        }
                                    })?;
                                if matches!(
                                    worker_persona.agent_type.as_deref(),
                                    Some(kind) if !kind.eq_ignore_ascii_case("worker")
                                ) {
                                    return Err(WorkflowError::StepFailed {
                                        id: step.id.clone(),
                                        reason: format!(
                                            "worker persona '{}' must have agent_type = 'worker'",
                                            worker.name
                                        ),
                                    });
                                }
                            }

                            // Construct a memory store keyed by persona namespace.
                            let memory_dir = agent007_core::paths::agent007_home().join("memory");
                            if let Err(e) = std::fs::create_dir_all(&memory_dir) {
                                tracing::warn!(
                                    error = %e,
                                    path = %memory_dir.display(),
                                    "failed to create memory directory for multi-agent step"
                                );
                            }
                            let mem_store =
                                Arc::new(agent007_memory::store::MemoryStore::new(&memory_dir));
                            let ns = persona_spec
                                .memory_namespace
                                .clone()
                                .unwrap_or_else(|| persona_spec.name.clone());
                            let scoped = Arc::new(mem_store.scoped(&ns));

                            // Build and run the sub-orchestrator.
                            let orchestrator =
                                agent007_custom_agents::SubOrchestrator::from_persona(
                                    &persona_spec,
                                    worker_specs,
                                    skill_provider_for_step,
                                    scoped,
                                    router.clone(),
                                    persona_provider.clone(),
                                    sub_dispatcher.clone(),
                                    0,
                                    3,
                                );

                            let result = orchestrator.run(&task_str).await.map_err(|e| {
                                WorkflowError::StepFailed {
                                    id: step.id.clone(),
                                    reason: format!("multi-agent step failed: {e}"),
                                }
                            })?;

                            // SubOrchestrator returns a conservative worker-call token estimate.
                            Ok((
                                step,
                                result.output,
                                format!("multi-agent/{persona_name}"),
                                result.token_estimate,
                            ))
                        }
                        .await;
                        step_futures.push(tokio::spawn(async move { multi_result }));
                        continue;
                    }

                    step_futures.push(tokio::spawn(async move {
                        // 1. Resolve prompt from inline or skill reference
                        let prompt_template = if let Some(ref prompt) = step.prompt {
                            prompt.clone()
                        } else if let Some(ref skill_trigger) = step.skill {
                            let skills_dir = agent007_core::paths::agent007_home().join("skills");
                            let loader = agent007_skills::SkillLoader::new(&skills_dir);
                            match loader.load_all() {
                                Ok(skills) => {
                                    match skills.into_iter().find(|s| s.trigger() == skill_trigger)
                                    {
                                        Some(skill) => skill.template().to_string(),
                                        None => {
                                            return Err(WorkflowError::SkillNotFound(
                                                skill_trigger.clone(),
                                            ))
                                        }
                                    }
                                }
                                Err(e) => {
                                    return Err(WorkflowError::StepFailed {
                                        id: step.id.clone(),
                                        reason: format!("failed to load skills: {e}"),
                                    })
                                }
                            }
                        } else {
                            return Err(WorkflowError::StepFailed {
                                id: step.id.clone(),
                                reason: "step must have either 'prompt' or 'skill'".to_string(),
                            });
                        };
                        let rendered = render_prompt(&prompt_template, &task_str, &ctx_outputs)
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
                            messages: vec![Message {
                                role: Role::User,
                                content: rendered.clone(),
                            }],
                            max_tokens: None,
                            temperature: None,
                            system: None,
                        };
                        let resp =
                            router
                                .complete(req)
                                .await
                                .map_err(|e| WorkflowError::StepFailed {
                                    id: step.id.clone(),
                                    reason: e.to_string(),
                                })?;

                        // Use actual API token counts when available; fall back to char estimate.
                        let tokens = resp
                            .input_tokens
                            .and_then(|i| resp.output_tokens.map(|o| (i + o) as usize))
                            .unwrap_or_else(|| rendered.len() / 4 + resp.content.len() / 4);
                        let actual_model = if resp.model.is_empty() {
                            model_name
                        } else {
                            resp.model.clone()
                        };

                        Ok::<(crate::types::StepDef, String, String, usize), WorkflowError>((
                            step,
                            resp.content,
                            actual_model,
                            tokens,
                        ))
                    }));
                }

                for fut in step_futures {
                    let (step, content, step_model, step_tokens) = match fut.await {
                        Ok(Ok(result)) => result,
                        Ok(Err(error)) => {
                            let step_id = workflow_error_step_id(&error);
                            if reliability_policy.recovery_enabled {
                                if let Some(ref failed_step_id) = step_id {
                                    let attempt = {
                                        let count = recovery_retry_counts
                                            .entry(failed_step_id.clone())
                                            .or_insert(0);
                                        *count += 1;
                                        *count
                                    };
                                    let max = reliability_policy.max_step_retries.max(1);

                                    self.trace_note(
                                        "workflow-step-retry",
                                        serde_json::json!({
                                            "workflow": def.name,
                                            "step_id": failed_step_id,
                                            "attempt": attempt,
                                            "max_retries": max,
                                            "reason": "step-execution-error",
                                            "error": error.to_string(),
                                        }),
                                    );
                                    let retry_transition = ReliabilityTransition::new(
                                        failed_step_id.clone(),
                                        ReliabilityTransitionKind::Retry,
                                        "step-execution-error-retry",
                                        Some(format!("attempt {attempt} of {max}")),
                                    );
                                    state.record_reliability_transition(retry_transition.clone());
                                    self.trace_note(
                                        "workflow-reliability-transition",
                                        serde_json::json!({
                                            "workflow": def.name,
                                            "transition": retry_transition,
                                        }),
                                    );
                                    state.mark_step_recovery_retry(failed_step_id, attempt);
                                    state.retry_counts = evaluator_retry_counts.clone();
                                    state.recovery_retry_counts = recovery_retry_counts.clone();
                                    state.sync_degradation_count(degradation_count);
                                    self.persist_workflow_artifacts(&state);

                                    if attempt <= max {
                                        rewind_target = Some(failed_step_id.clone());
                                        rewind_to_batch = step_batches.get(failed_step_id).copied();
                                        continue;
                                    }

                                    let abort_transition = ReliabilityTransition::new(
                                        failed_step_id.clone(),
                                        ReliabilityTransitionKind::Abort,
                                        "step-execution-max-retries-exceeded",
                                        Some(error.to_string()),
                                    );
                                    state.record_reliability_transition(abort_transition.clone());
                                    self.trace_note(
                                        "workflow-reliability-transition",
                                        serde_json::json!({
                                            "workflow": def.name,
                                            "transition": abort_transition,
                                        }),
                                    );
                                    return fail_workflow(
                                        self,
                                        &mut state,
                                        Some(failed_step_id.clone()),
                                        WorkflowError::MaxRetriesExceeded {
                                            id: failed_step_id.clone(),
                                            max,
                                        },
                                    );
                                }
                            }

                            if let Some(ref failed_step_id) = step_id {
                                let transition = ReliabilityTransition::new(
                                    failed_step_id.clone(),
                                    ReliabilityTransitionKind::Abort,
                                    "step-execution-error",
                                    Some(error.to_string()),
                                );
                                state.record_reliability_transition(transition.clone());
                                self.trace_note(
                                    "workflow-reliability-transition",
                                    serde_json::json!({
                                        "workflow": def.name,
                                        "transition": transition,
                                    }),
                                );
                            }
                            return fail_workflow(self, &mut state, step_id, error);
                        }
                        Err(error) => {
                            let transition = ReliabilityTransition::new(
                                "unknown",
                                ReliabilityTransitionKind::Abort,
                                "step-task-join-error",
                                Some(error.to_string()),
                            );
                            state.record_reliability_transition(transition.clone());
                            self.trace_note(
                                "workflow-reliability-transition",
                                serde_json::json!({
                                    "workflow": def.name,
                                    "transition": transition,
                                }),
                            );
                            return fail_workflow(
                                self,
                                &mut state,
                                None,
                                WorkflowError::StepFailed {
                                    id: "unknown".to_string(),
                                    reason: error.to_string(),
                                },
                            );
                        }
                    };
                    // Emit a ModelRequest event so the dashboard picks up per-step tokens/model.
                    let _ = self
                        .dispatcher
                        .publish(AgentEvent::ModelRequest {
                            provider: step_model,
                            prompt_ref: PromptRef::new(),
                            token_estimate: step_tokens,
                        })
                        .await;

                    let escalation = evaluate_confidence(&content, &reliability_policy);
                    let force_approval =
                        matches!(escalation, EscalationDecision::RequestApproval { .. });
                    if let EscalationDecision::RequestApproval { reason_code } = &escalation {
                        let transition = ReliabilityTransition::new(
                            step.id.clone(),
                            ReliabilityTransitionKind::EscalateApproval,
                            reason_code.clone(),
                            Some("confidence policy requested approval".to_string()),
                        );
                        state.record_reliability_transition(transition.clone());
                        self.trace_note(
                            "workflow-confidence-escalation",
                            serde_json::json!({
                                "workflow": def.name,
                                "step_id": step.id,
                                "decision": "request-approval",
                                "reason_code": reason_code,
                            }),
                        );
                        self.trace_note(
                            "workflow-reliability-transition",
                            serde_json::json!({
                                "workflow": def.name,
                                "transition": transition,
                            }),
                        );
                    }

                    // Handle approval gate (sequential after the step completes)
                    let mut final_content = match self
                        .resolve_approval_decision(&mut state, &step, &content, force_approval)
                        .await
                    {
                        Ok(content) => content,
                        Err(error) => {
                            return fail_workflow(self, &mut state, Some(step.id.clone()), error);
                        }
                    };

                    // Enforce budget and apply graceful degradation when configured.
                    if let Some(budget) = &def.budget {
                        let mut used = budget_used.lock().await;
                        let token_estimate = estimate_tokens(&final_content);
                        let usd_estimate = token_estimate as f64 * 0.000_002; // $2 per 1M tokens placeholder

                        match evaluate_budget_decision(
                            budget,
                            &used,
                            token_estimate,
                            usd_estimate,
                            degradation_count,
                            &reliability_policy,
                        ) {
                            BudgetDecision::Continue { .. } => {
                                used.tokens += token_estimate;
                                used.estimated_usd += usd_estimate;
                                if let Err(error) = check_budget(budget, &used) {
                                    let transition = ReliabilityTransition::new(
                                        step.id.clone(),
                                        ReliabilityTransitionKind::Abort,
                                        "budget-limit-exceeded",
                                        Some(error.to_string()),
                                    );
                                    state.record_reliability_transition(transition.clone());
                                    self.trace_note(
                                        "workflow-reliability-transition",
                                        serde_json::json!({
                                            "workflow": def.name,
                                            "transition": transition,
                                        }),
                                    );
                                    drop(used);
                                    return fail_workflow(
                                        self,
                                        &mut state,
                                        Some(step.id.clone()),
                                        error,
                                    );
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
                                    let transition = ReliabilityTransition::new(
                                        step.id.clone(),
                                        ReliabilityTransitionKind::Abort,
                                        "budget-degrade-failed",
                                        Some(error.to_string()),
                                    );
                                    state.record_reliability_transition(transition.clone());
                                    self.trace_note(
                                        "workflow-reliability-transition",
                                        serde_json::json!({
                                            "workflow": def.name,
                                            "transition": transition,
                                        }),
                                    );
                                    drop(used);
                                    return fail_workflow(
                                        self,
                                        &mut state,
                                        Some(step.id.clone()),
                                        error,
                                    );
                                }

                                final_content = degraded;
                                *used = projected;
                                degradation_count = degradation_count.saturating_add(1);
                                state.sync_degradation_count(degradation_count);

                                let transition = ReliabilityTransition::new(
                                    step.id.clone(),
                                    ReliabilityTransitionKind::Degrade,
                                    reason_code,
                                    Some(format!(
                                        "output truncated to {} chars to remain within budget",
                                        target_chars
                                    )),
                                );
                                state.record_reliability_transition(transition.clone());
                                self.trace_note(
                                    "workflow-budget-decision",
                                    serde_json::json!({
                                        "workflow": def.name,
                                        "step_id": step.id,
                                        "decision": "degrade",
                                        "degradation_count": degradation_count,
                                        "target_chars": target_chars,
                                    }),
                                );
                                self.trace_note(
                                    "workflow-reliability-transition",
                                    serde_json::json!({
                                        "workflow": def.name,
                                        "transition": transition,
                                    }),
                                );
                            }
                            BudgetDecision::Abort { reason_code } => {
                                let transition = ReliabilityTransition::new(
                                    step.id.clone(),
                                    ReliabilityTransitionKind::Abort,
                                    reason_code.clone(),
                                    Some("budget governor aborted execution".to_string()),
                                );
                                state.record_reliability_transition(transition.clone());
                                self.trace_note(
                                    "workflow-reliability-transition",
                                    serde_json::json!({
                                        "workflow": def.name,
                                        "transition": transition,
                                    }),
                                );
                                drop(used);
                                return fail_workflow(
                                    self,
                                    &mut state,
                                    Some(step.id.clone()),
                                    WorkflowError::BudgetExceeded(reason_code),
                                );
                            }
                        }
                    }

                    // Store output artifact
                    if let Some(out_name) = &step.output {
                        outputs
                            .lock()
                            .await
                            .insert(out_name.clone(), final_content.clone());
                    }
                    state.mark_step_completed(&step, &final_content);
                    state.sync_outputs(outputs.lock().await.clone());
                    state.sync_budget(budget_used.lock().await.clone());
                    state.sync_degradation_count(degradation_count);
                    let transition = ReliabilityTransition::new(
                        step.id.clone(),
                        ReliabilityTransitionKind::Continue,
                        "step-completed",
                        None,
                    );
                    state.record_reliability_transition(transition.clone());
                    self.trace_note(
                        "workflow-step-completed",
                        serde_json::json!({
                            "workflow": def.name,
                            "step_id": step.id,
                            "output": step.output,
                            "output_preview": preview(&final_content),
                            "requires_approval": step.requires_approval.unwrap_or(false),
                        }),
                    );
                    self.trace_note(
                        "workflow-reliability-transition",
                        serde_json::json!({
                            "workflow": def.name,
                            "transition": transition,
                        }),
                    );

                    match step.r#type {
                        StepType::Execute => {}
                        StepType::Evaluator => {
                            if let Some(eval) = &step.evaluate {
                                let current = outputs.lock().await.clone();
                                let passed = if let Some(cond) = &eval.condition {
                                    evaluate_condition(cond, &current)
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
                                self.trace_note(
                                    "workflow-evaluator-result",
                                    serde_json::json!({
                                        "workflow": def.name,
                                        "step_id": step.id,
                                        "passed": passed,
                                        "selected": selected_target,
                                    }),
                                );
                                if !passed {
                                    let attempt = {
                                        let count = evaluator_retry_counts
                                            .entry(step.id.clone())
                                            .or_insert(0);
                                        *count += 1;
                                        *count
                                    };
                                    let max = eval.max_retries.unwrap_or(3);
                                    self.trace_note(
                                        "workflow-step-retry",
                                        serde_json::json!({
                                            "workflow": def.name,
                                            "step_id": step.id,
                                            "attempt": attempt,
                                            "max_retries": max,
                                        }),
                                    );
                                    let retry_transition = ReliabilityTransition::new(
                                        step.id.clone(),
                                        ReliabilityTransitionKind::Retry,
                                        "evaluator-failed-retry",
                                        Some(format!("attempt {attempt} of {max}")),
                                    );
                                    state.record_reliability_transition(retry_transition.clone());
                                    self.trace_note(
                                        "workflow-reliability-transition",
                                        serde_json::json!({
                                            "workflow": def.name,
                                            "transition": retry_transition,
                                        }),
                                    );
                                    state.mark_step_retry(&step.id, attempt);
                                    state.retry_counts = evaluator_retry_counts.clone();
                                    state.recovery_retry_counts = recovery_retry_counts.clone();
                                    self.persist_workflow_artifacts(&state);
                                    if attempt >= max {
                                        let abort_transition = ReliabilityTransition::new(
                                            step.id.clone(),
                                            ReliabilityTransitionKind::Abort,
                                            "max-retries-exceeded",
                                            Some(format!("attempt {attempt} reached max {max}")),
                                        );
                                        state.record_reliability_transition(
                                            abort_transition.clone(),
                                        );
                                        self.trace_note(
                                            "workflow-reliability-transition",
                                            serde_json::json!({
                                                "workflow": def.name,
                                                "transition": abort_transition,
                                            }),
                                        );
                                        return fail_workflow(
                                            self,
                                            &mut state,
                                            Some(step.id.clone()),
                                            WorkflowError::MaxRetriesExceeded {
                                                id: step.id.clone(),
                                                max,
                                            },
                                        );
                                    }
                                    rewind_target = Some(eval.on_fail.clone());
                                    rewind_to_batch = step_batches.get(&eval.on_fail).copied();
                                } else {
                                    for dependent in
                                        step_dependents.get(&step.id).into_iter().flatten()
                                    {
                                        if dependent != &eval.on_pass {
                                            skipped_steps.insert(dependent.clone());
                                            state.mark_step_skipped(dependent);
                                        }
                                    }
                                }
                            }
                        }
                        StepType::Router => {
                            if let Some(routes) = &step.routes {
                                match match_route(&final_content, routes) {
                                    Some(goto) => {
                                        if let Some(store) = &self.run_store {
                                            let candidates = routes
                                                .iter()
                                                .map(|route| route.goto.clone())
                                                .collect::<Vec<_>>();
                                            let recommendation = recommend_route_for_step(
                                                store,
                                                &def.name,
                                                &step.id,
                                                goto,
                                                &candidates,
                                                self.run_id.as_deref(),
                                            );
                                            state.record_routing_recommendation(
                                                recommendation.clone(),
                                            );
                                            self.trace_note(
                                                "workflow-routing-recommendation",
                                                serde_json::json!({
                                                    "workflow": def.name,
                                                    "step_id": step.id,
                                                    "current_route": recommendation.current_route,
                                                    "recommended_route": recommendation.recommended_route,
                                                    "confidence": recommendation.confidence,
                                                    "fallback_used": recommendation.fallback_used,
                                                    "sample_size": recommendation.sample_size,
                                                }),
                                            );
                                        }
                                        self.trace_note(
                                            "workflow-route-selected",
                                            serde_json::json!({
                                                "workflow": def.name,
                                                "step_id": step.id,
                                                "selected": goto,
                                            }),
                                        );
                                        state.mark_route_selected(&step.id, goto);
                                        for route in routes {
                                            if route.goto != goto {
                                                skipped_steps.insert(route.goto.clone());
                                                state.mark_step_skipped(&route.goto);
                                            }
                                        }
                                    }
                                    None => {
                                        return fail_workflow(
                                            self,
                                            &mut state,
                                            Some(step.id.clone()),
                                            WorkflowError::NoRouteMatch {
                                                id: step.id.clone(),
                                                output: final_content,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        // Sub-workflow outputs were already injected during execution;
                        // nothing extra to do at the post-step routing stage.
                        StepType::SubWorkflow => {}
                        // Extract steps are handled inline in the hosted engine; the
                        // parallel runner does not support them and skips post-processing.
                        StepType::Extract => {}
                        // MultiAgent steps are dispatched via SubOrchestrator; handled
                        // in the execution branch below. Nothing extra at post-step stage.
                        StepType::MultiAgent => {}
                    }

                    completed_steps.insert(step.id.clone());
                    steps_completed += 1;
                    state.completed_steps = completed_steps.iter().cloned().collect();
                    state.completed_steps.sort();
                    state.steps_completed = steps_completed;
                    state.skipped_steps = skipped_steps.iter().cloned().collect();
                    state.skipped_steps.sort();
                    state.retry_counts = evaluator_retry_counts.clone();
                    state.recovery_retry_counts = recovery_retry_counts.clone();
                    state.sync_outputs(outputs.lock().await.clone());
                    state.sync_budget(budget_used.lock().await.clone());
                    state.sync_degradation_count(degradation_count);
                    self.persist_workflow_artifacts(&state);
                }

                if let (Some(target), Some(rewind_batch)) = (rewind_target, rewind_to_batch) {
                    let reset_ids = {
                        let mut current_outputs = outputs.lock().await;
                        reset_steps_from_target(
                            &target,
                            &step_dependents,
                            &step_map,
                            &mut completed_steps,
                            &mut skipped_steps,
                            &mut state,
                            &mut current_outputs,
                        )
                    };
                    steps_completed = completed_steps.len();
                    state.completed_steps = completed_steps.iter().cloned().collect();
                    state.completed_steps.sort();
                    state.skipped_steps = skipped_steps.iter().cloned().collect();
                    state.skipped_steps.sort();
                    state.steps_completed = steps_completed;
                    state.sync_outputs(outputs.lock().await.clone());
                    state.sync_budget(budget_used.lock().await.clone());
                    state.sync_degradation_count(degradation_count);
                    self.persist_workflow_artifacts(&state);
                    self.trace_note(
                        "workflow-rewind",
                        serde_json::json!({
                            "workflow": def.name,
                            "target": target,
                            "batch_index": rewind_batch,
                            "reset_steps": reset_ids,
                        }),
                    );
                    batch_index = rewind_batch;
                    continue 'workflow_batches;
                }
            }
            batch_index += 1;
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

        state.mark_succeeded();
        state.sync_outputs(final_outputs.clone());
        state.sync_budget(final_budget.clone());
        state.sync_degradation_count(degradation_count);
        if let Err(error) = self.apply_eval_gate(def, &mut state, &final_budget) {
            return fail_workflow(self, &mut state, Some("eval-gate".to_string()), error);
        }
        if state.status == crate::state::WorkflowRunStatus::Failed {
            self.persist_workflow_artifacts(&state);
            return Err(WorkflowError::EvalGateBlocked {
                workflow: def.name.clone(),
                reason: state
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "eval gate blocked workflow".to_string()),
            });
        }
        self.persist_workflow_artifacts(&state);
        self.trace_note(
            "workflow-complete",
            serde_json::json!({
                "workflow": def.name,
                "steps_completed": steps_completed,
                "steps_total": steps_total,
                "outputs": final_outputs.keys().cloned().collect::<Vec<_>>(),
                "budget_used": final_budget.clone(),
            }),
        );

        Ok(WorkflowResult {
            outputs: final_outputs,
            steps_completed,
            steps_total,
            budget_used: final_budget,
        })
    }

    fn persist_workflow_artifacts(&self, state: &WorkflowRunState) {
        if let (Some(store), Some(run_id)) = (&self.run_store, &self.run_id) {
            let _ = store.write_json_artifact(run_id, "workflow-request.json", &state.request());
            let _ = store.write_json_artifact(run_id, "workflow-state.json", state);
            if !state.routing_recommendations.is_empty() {
                let _ = store.write_json_artifact(
                    run_id,
                    "routing-recommendations.json",
                    &state.routing_recommendations,
                );
            }
            if let Some(decision) = &state.eval_gate_decision {
                let _ = persist_eval_gate_artifacts(store, run_id, decision);
            }
        }
    }

    fn apply_eval_gate(
        &self,
        def: &WorkflowDef,
        state: &mut WorkflowRunState,
        final_budget: &BudgetUsed,
    ) -> Result<(), WorkflowError> {
        let Some(store) = &self.run_store else {
            return Ok(());
        };
        let Some(run_id) = &self.run_id else {
            return Ok(());
        };

        let policy = EvalGatePolicy::from_workflow(def);
        let Some(decision) =
            evaluate_workflow_eval_gate(store, run_id, &def.name, final_budget, &policy)?
        else {
            return Ok(());
        };

        state.set_eval_gate_decision(decision.clone());
        match decision.decision {
            WorkflowEvalGateDecisionKind::Pass => {
                self.trace_note(
                    "workflow-eval-baseline",
                    serde_json::json!({
                        "workflow": def.name,
                        "baseline_sample_size": decision.baseline_sample_size,
                        "decision": "pass",
                    }),
                );
            }
            WorkflowEvalGateDecisionKind::Warn => {
                self.trace_note(
                    "workflow-eval-baseline",
                    serde_json::json!({
                        "workflow": def.name,
                        "baseline_sample_size": decision.baseline_sample_size,
                        "decision": "warn",
                        "reason_codes": decision.reason_codes,
                    }),
                );
            }
            WorkflowEvalGateDecisionKind::Block => {
                state.mark_failed(
                    None,
                    format!(
                        "eval gate blocked workflow '{}': {}",
                        def.name, decision.message
                    ),
                );
            }
        }
        Ok(())
    }

    async fn resolve_approval_decision(
        &self,
        state: &mut WorkflowRunState,
        step: &crate::types::StepDef,
        content: &str,
        force_approval: bool,
    ) -> Result<String, WorkflowError> {
        if !step.requires_approval.unwrap_or(false) && !force_approval {
            return Ok(content.to_string());
        }

        let decision = if let Some(existing) = state.approval_decision(&step.id) {
            existing
        } else if std::env::var("AGENT007_AUTO_APPROVE")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            ApprovalDecision {
                decision: ApprovalDecisionKind::Approve,
                content: Some(content.to_string()),
            }
        } else if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
            ApprovalGate::prompt(&step.id, content).await?
        } else {
            state.mark_step_awaiting_approval(step, content);
            self.persist_workflow_artifacts(state);
            self.trace_note(
                "workflow-approval-required",
                serde_json::json!({
                    "workflow": state.workflow,
                    "step_id": step.id,
                    "agent": step.agent,
                    "output": step.output,
                    "output_preview": preview(content),
                    "trigger": if force_approval { "confidence-escalation" } else { "step-config" },
                }),
            );
            return Err(WorkflowError::ApprovalRequired {
                id: step.id.clone(),
            });
        };

        state.record_approval_decision(&step.id, decision.clone());
        self.persist_workflow_artifacts(state);
        self.trace_note(
            "workflow-approval-decision",
            serde_json::json!({
                "workflow": state.workflow,
                "step_id": step.id,
                "decision": decision.decision,
            }),
        );

        match decision.decision {
            ApprovalDecisionKind::Approve => {
                Ok(decision.content.unwrap_or_else(|| content.to_string()))
            }
            ApprovalDecisionKind::Edit => {
                Ok(decision.content.unwrap_or_else(|| content.to_string()))
            }
            ApprovalDecisionKind::Deny => Err(WorkflowError::ApprovalDenied(step.id.clone())),
        }
    }

    fn trace_note(&self, kind: &str, payload: serde_json::Value) {
        if let (Some(store), Some(run_id)) = (&self.run_store, &self.run_id) {
            let _ = store.append_note(run_id, kind, payload);
        }
    }
}

fn workflow_error_step_id(error: &WorkflowError) -> Option<String> {
    match error {
        WorkflowError::StepFailed { id, .. }
        | WorkflowError::TemplateError { id, .. }
        | WorkflowError::MaxRetriesExceeded { id, .. }
        | WorkflowError::NoRouteMatch { id, .. }
        | WorkflowError::InvalidEvaluator { id, .. }
        | WorkflowError::InvalidRouter { id, .. } => Some(id.clone()),
        WorkflowError::ApprovalDenied(id) => Some(id.clone()),
        WorkflowError::ApprovalRequired { id } => Some(id.clone()),
        _ => None,
    }
}

fn build_step_batch_map(batches: &[Vec<String>]) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for (batch_index, batch) in batches.iter().enumerate() {
        for step_id in batch {
            map.insert(step_id.clone(), batch_index);
        }
    }
    map
}

pub(crate) fn build_step_dependents(
    def: &WorkflowDef,
    step_map: &HashMap<String, crate::types::StepDef>,
) -> HashMap<String, Vec<String>> {
    let mut output_to_step = HashMap::new();
    for step in def.steps.iter() {
        if let Some(output) = &step.output {
            output_to_step.insert(output.clone(), step.id.clone());
        }
    }

    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    for step in step_map.values() {
        for dep in step.depends_on.iter().flatten() {
            dependents
                .entry(dep.clone())
                .or_default()
                .push(step.id.clone());
        }
        for input in step.inputs.iter().flatten() {
            if let Some(producer) = output_to_step.get(input) {
                dependents
                    .entry(producer.clone())
                    .or_default()
                    .push(step.id.clone());
            }
        }
    }

    for entries in dependents.values_mut() {
        entries.sort();
        entries.dedup();
    }

    dependents
}

pub(crate) fn reset_steps_from_target(
    target: &str,
    step_dependents: &HashMap<String, Vec<String>>,
    step_map: &HashMap<String, crate::types::StepDef>,
    completed_steps: &mut HashSet<String>,
    skipped_steps: &mut HashSet<String>,
    state: &mut WorkflowRunState,
    outputs: &mut HashMap<String, String>,
) -> Vec<String> {
    let mut stack = vec![target.to_string()];
    let mut visited = HashSet::new();

    while let Some(step_id) = stack.pop() {
        if !visited.insert(step_id.clone()) {
            continue;
        }
        if let Some(dependents) = step_dependents.get(&step_id) {
            stack.extend(dependents.iter().cloned());
        }
    }

    for step_id in &visited {
        completed_steps.remove(step_id);
        skipped_steps.remove(step_id);
        state.reset_step(step_id);
        if let Some(step) = step_map.get(step_id) {
            if let Some(output_key) = &step.output {
                outputs.remove(output_key);
            }
        }
    }

    let mut reset_ids = visited.into_iter().collect::<Vec<_>>();
    reset_ids.sort();
    reset_ids
}

fn fail_workflow(
    runner: &WorkflowRunner,
    state: &mut WorkflowRunState,
    step_id: Option<String>,
    error: WorkflowError,
) -> Result<WorkflowResult, WorkflowError> {
    match &error {
        WorkflowError::ApprovalRequired { .. } => {
            runner.persist_workflow_artifacts(state);
        }
        _ => {
            state.mark_failed(step_id.as_deref(), error.to_string());
            runner.persist_workflow_artifacts(state);
        }
    }
    Err(error)
}

pub(crate) fn render_prompt(
    template: &str,
    task: &str,
    outputs: &HashMap<String, String>,
) -> Result<String, tera::Error> {
    let mut tera = tera::Tera::default();
    tera.add_raw_template("prompt", template)?;
    let mut ctx = tera::Context::new();
    ctx.insert("task", task);
    ctx.insert("args", task);
    // Workflows and skills commonly reference memory/rag placeholders. When a
    // caller has not pre-injected them yet, render them as empty strings
    // instead of aborting the entire step on a missing Tera variable.
    let mut memory = HashMap::new();
    memory.insert(
        "project",
        outputs
            .get("memory.project")
            .map(String::as_str)
            .unwrap_or(""),
    );
    memory.insert(
        "user",
        outputs.get("memory.user").map(String::as_str).unwrap_or(""),
    );
    memory.insert(
        "global",
        outputs
            .get("memory.global")
            .map(String::as_str)
            .unwrap_or(""),
    );
    memory.insert(
        "repo_brain",
        outputs
            .get("memory.repo_brain")
            .map(String::as_str)
            .unwrap_or(""),
    );
    ctx.insert("memory", &memory);
    ctx.insert(
        "rag_context",
        outputs.get("rag_context").map(String::as_str).unwrap_or(""),
    );

    for (k, v) in outputs {
        match k.as_str() {
            "task" | "args" | "memory" | "memory.project" | "memory.user" | "memory.global"
            | "memory.repo_brain" | "rag_context" => {}
            _ => {
                let minified = minify_context(v);
                ctx.insert(k, &minified);
            }
        }
    }
    tera.render("prompt", &ctx)
}

pub(crate) fn estimate_tokens(text: &str) -> u64 {
    // Rough approximation: 1 token ≈ 4 chars
    (text.len() as u64) / 4
}

/// Losslessly minify Markdown context to reduce injected token count.
///
/// Skips structured data (JSON objects/arrays and content with code fences) entirely,
/// since whitespace is often significant there and token savings are minimal.
pub(crate) fn minify_context(text: &str) -> String {
    // Guard: skip structured data unchanged
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return text.to_string();
    }
    if text.contains("```\n") || text.contains("```\r\n") {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut blank_run: u32 = 0;
    let mut prev_was_hr = false;

    for line in text.lines() {
        let line = line.trim_end();

        if line.is_empty() {
            blank_run += 1;
            // Allow at most 1 consecutive blank line (2 newlines in output)
            if blank_run <= 1 {
                result.push('\n');
            }
            continue;
        }
        blank_run = 0;

        // Remove single-line HTML comments
        if line.starts_with("<!--") && line.ends_with("-->") {
            continue;
        }

        // Collapse repeated Markdown horizontal rules
        let is_hr = line == "---" || line == "***" || line == "___";
        if is_hr && prev_was_hr {
            continue;
        }
        prev_was_hr = is_hr;

        // Normalize multiple spaces to single (only on non-indented lines)
        let line_out =
            if line.contains("  ") && line.chars().next().map_or(false, |c| !c.is_whitespace()) {
                std::borrow::Cow::Owned(line.split_whitespace().collect::<Vec<_>>().join(" "))
            } else {
                std::borrow::Cow::Borrowed(line)
            };

        result.push_str(&line_out);
        result.push('\n');
    }

    result
}

pub(crate) fn preview(text: &str) -> String {
    const MAX_CHARS: usize = 160;
    let preview: String = text.chars().take(MAX_CHARS).collect();
    if text.chars().count() > MAX_CHARS {
        format!("{preview}...")
    } else {
        preview
    }
}

pub(crate) fn check_budget(
    budget: &crate::types::BudgetConfig,
    used: &BudgetUsed,
) -> Result<(), WorkflowError> {
    let mode = budget.on_exceed.as_deref().unwrap_or("stop");

    let token_exceeded = budget
        .max_tokens_per_session
        .map_or(false, |max| used.tokens > max);
    let usd_exceeded = budget
        .max_usd_per_task
        .map_or(false, |max| used.estimated_usd > max);

    if !token_exceeded && !usd_exceeded {
        // Check alert threshold
        if let (Some(alert_pct), Some(max_tokens)) =
            (budget.alert_at_percent, budget.max_tokens_per_session)
        {
            let pct_used = (used.tokens as f64 / max_tokens as f64) * 100.0;
            if pct_used >= alert_pct as f64 {
                tracing::warn!(
                    "Budget alert: {:.0}% of token limit used ({}/{})",
                    pct_used,
                    used.tokens,
                    max_tokens
                );
            }
        }
        return Ok(());
    }

    let reason = if token_exceeded {
        format!(
            "token limit {} exceeded (used {})",
            budget.max_tokens_per_session.unwrap(),
            used.tokens
        )
    } else {
        format!(
            "USD limit ${:.6} exceeded (used ${:.6})",
            budget.max_usd_per_task.unwrap(),
            used.estimated_usd
        )
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

pub(crate) fn step_is_ready(
    step: &crate::types::StepDef,
    outputs: &HashMap<String, String>,
    completed_steps: &HashSet<String>,
) -> bool {
    let deps_ok = step
        .depends_on
        .iter()
        .flatten()
        .all(|d| completed_steps.contains(d));
    let inputs_ok = step
        .inputs
        .iter()
        .flatten()
        .all(|inp| outputs.contains_key(inp));
    deps_ok && inputs_ok
}

pub(crate) fn evaluate_condition(condition: &str, outputs: &HashMap<String, String>) -> bool {
    let mut rendered = condition.to_string();
    for (k, v) in outputs {
        rendered = rendered.replace(&format!("{{{{{k}}}}}"), v);
    }

    if let Some((lhs, rhs)) = rendered.split_once(" contains ") {
        let rhs = rhs.trim().trim_matches('\'').trim_matches('"');
        return lhs.to_lowercase().contains(&rhs.to_lowercase());
    }
    if let Some((lhs, rhs)) = rendered.split_once(" equals ") {
        let rhs = rhs.trim().trim_matches('\'').trim_matches('"');
        return lhs.trim().eq_ignore_ascii_case(rhs);
    }
    if let Some((lhs, rhs)) = rendered.split_once(" starts_with ") {
        let rhs = rhs.trim().trim_matches('\'').trim_matches('"');
        return lhs.to_lowercase().starts_with(&rhs.to_lowercase());
    }
    if let Some((lhs, rhs)) = rendered.split_once(" not_contains ") {
        let rhs = rhs.trim().trim_matches('\'').trim_matches('"');
        return !lhs.to_lowercase().contains(&rhs.to_lowercase());
    }

    false
}

pub(crate) fn evaluate_decision_field(output: &str, field: &str) -> bool {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
        if let Some(val) = json.get(field).and_then(|v| v.as_str()) {
            return val.eq_ignore_ascii_case("pass");
        }
    }
    false
}

pub(crate) fn match_route<'a>(
    output: &str,
    routes: &'a [crate::types::RouteConfig],
) -> Option<&'a str> {
    let normalized = output.trim().to_lowercase();
    for route in routes {
        if let Some(when) = &route.when {
            if normalized.contains(&when.to_lowercase()) {
                return Some(&route.goto);
            }
        }
    }
    for route in routes {
        if route.default {
            return Some(&route.goto);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        BudgetConfig, EvalGateConfig, EvalGateMode, EvalGateThresholdConfig, EvaluateConfig,
        RouteConfig, StepDef, StepType, WorkflowDef,
    };
    use agent007_core::dispatcher::LocalDispatcher;
    use agent007_core::persona::NoOpPersonaProvider;
    use agent007_core::{RunScorecard, RunStatus, RunStore};
    use agent007_models::{
        CompletionRequest, CompletionResponse, MockProvider, ModelError, ModelProvider, ModelRouter,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

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

    struct SequenceProvider {
        responses: Arc<StdMutex<VecDeque<String>>>,
    }

    impl SequenceProvider {
        fn new(responses: &[&str]) -> Self {
            Self {
                responses: Arc::new(StdMutex::new(
                    responses.iter().map(|value| value.to_string()).collect(),
                )),
            }
        }
    }

    #[async_trait]
    impl ModelProvider for SequenceProvider {
        fn name(&self) -> &str {
            "sequence"
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, ModelError> {
            let content = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| "sequence-empty".to_string());
            Ok(CompletionResponse {
                content,
                model: "sequence".to_string(),
                input_tokens: None,
                output_tokens: None,
                cached_tokens: None,
            })
        }
    }

    fn sequence_runner(responses: &[&str]) -> WorkflowRunner {
        let provider = Arc::new(SequenceProvider::new(responses));
        let mut router = ModelRouter::new("mock");
        router.register("mock", provider as Arc<dyn ModelProvider>);
        let dispatcher = LocalDispatcher::new(32);
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
                prompt: Some("research {{task}}".to_string()),
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

    #[test]
    fn render_prompt_defaults_missing_memory_and_rag_context() {
        let outputs = HashMap::new();
        let rendered = render_prompt(
            "Task={{task}}\nProject={{memory.project}}\nBrain={{memory.repo_brain}}\nRag={{rag_context}}",
            "review current diff",
            &outputs,
        )
        .expect("template should render");

        assert_eq!(rendered, "Task=review current diff\nProject=\nBrain=\nRag=");
    }

    #[test]
    fn render_prompt_uses_reserved_memory_and_rag_values_when_provided() {
        let outputs = HashMap::from([
            ("memory.project".to_string(), "project notes".to_string()),
            ("memory.repo_brain".to_string(), "repo brain".to_string()),
            ("rag_context".to_string(), "prior findings".to_string()),
        ]);
        let rendered = render_prompt(
            "Project={{memory.project}}\nBrain={{memory.repo_brain}}\nRag={{rag_context}}",
            "review current diff",
            &outputs,
        )
        .expect("template should render");

        assert_eq!(
            rendered,
            "Project=project notes\nBrain=repo brain\nRag=prior findings"
        );
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
                    prompt: Some("research {{task}}".to_string()),
                    skill: None,
                    output: Some("notes".to_string()),
                    requires_approval: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                    workflow: None,
                    ..Default::default()
                },
                StepDef {
                    id: "step2".to_string(),
                    agent: "Coder".to_string(),
                    model: None,
                    inputs: Some(vec!["notes".to_string()]),
                    depends_on: None,
                    prompt: Some("implement based on {{notes}}".to_string()),
                    skill: None,
                    output: Some("code".to_string()),
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
        }
    }

    fn release_gated_def(mode: EvalGateMode) -> WorkflowDef {
        WorkflowDef {
            name: "release-gated".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "ship".to_string(),
                agent: "Researcher".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("ship {{task}}".to_string()),
                skill: None,
                output: Some("notes".to_string()),
                requires_approval: None,
                r#type: StepType::Execute,
                evaluate: None,
                routes: None,
                workflow: None,
                ..Default::default()
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: Some(10_000),
                max_usd_per_task: Some(1.0),
                alert_at_percent: None,
                on_exceed: Some("alert-only".to_string()),
            }),
            reliability: None,
            eval_gate: Some(EvalGateConfig {
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
            }),
        }
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

    #[tokio::test]
    async fn run_single_step_returns_output() {
        let runner = mock_runner("mocked output");
        let result = runner.run(&simple_def(), "build auth").await.unwrap();
        assert_eq!(result.steps_completed, 1);
        assert_eq!(result.steps_total, 1);
        assert_eq!(
            result.outputs.get("notes").map(|s| s.as_str()),
            Some("mocked output")
        );
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
    async fn traced_workflow_records_step_notes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RunStore::new(dir.path()));
        let run = store
            .create_run("workflow-test", "build auth", "standalone", Some("mock"))
            .unwrap();
        let runner = mock_runner("mocked output").for_run(store.clone(), run.id.clone());

        let result = runner.run(&simple_def(), "build auth").await.unwrap();
        assert_eq!(result.steps_completed, 1);

        let detail = store.load_run(&run.id).unwrap();
        let kinds = detail
            .entries
            .iter()
            .map(|entry| entry.kind.as_str())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"workflow-start"));
        assert!(kinds.contains(&"workflow-step-dispatched"));
        assert!(kinds.contains(&"workflow-step-completed"));
        assert!(kinds.contains(&"workflow-complete"));
    }

    #[tokio::test]
    async fn validate_cycle_returns_error() {
        let def = WorkflowDef {
            name: "cycle".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "a".to_string(),
                    agent: "A".to_string(),
                    model: None,
                    inputs: Some(vec!["out_b".to_string()]),
                    depends_on: None,
                    prompt: Some("p".to_string()),
                    skill: None,
                    output: Some("out_a".to_string()),
                    requires_approval: None,
                    r#type: StepType::Execute,
                    evaluate: None,
                    routes: None,
                    workflow: None,
                    ..Default::default()
                },
                StepDef {
                    id: "b".to_string(),
                    agent: "B".to_string(),
                    model: None,
                    inputs: Some(vec!["out_a".to_string()]),
                    depends_on: None,
                    prompt: Some("p".to_string()),
                    skill: None,
                    output: Some("out_b".to_string()),
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
                id: "s".to_string(),
                agent: "A".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("task is {{task}}".to_string()),
                skill: None,
                output: None,
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
                id: "s1".to_string(),
                agent: "A".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("do {{task}}".to_string()),
                skill: None,
                output: Some("out".to_string()),
                requires_approval: None,
                r#type: StepType::Execute,
                evaluate: None,
                routes: None,
                workflow: None,
                ..Default::default()
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: Some(1), // extremely low — 1 token
                max_usd_per_task: None,
                alert_at_percent: None,
                on_exceed: Some("stop".to_string()),
            }),
            reliability: None,
            eval_gate: None,
        };
        let err = runner.run(&def, "task").await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::BudgetExceeded(_)
        ));
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
                id: "s1".to_string(),
                agent: "A".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("do {{task}}".to_string()),
                skill: None,
                output: Some("out".to_string()),
                requires_approval: None,
                r#type: StepType::Execute,
                evaluate: None,
                routes: None,
                workflow: None,
                ..Default::default()
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: None,
                max_usd_per_task: Some(0.000_000_001), // sub-nano USD — always exceeded
                alert_at_percent: None,
                on_exceed: Some("stop".to_string()),
            }),
            reliability: None,
            eval_gate: None,
        };
        let err = runner.run(&def, "task").await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::BudgetExceeded(_)
        ));
    }

    #[tokio::test]
    async fn budget_alert_only_does_not_stop_run() {
        let runner = mock_runner("a very long output that is definitely more than 1 token");
        let def = WorkflowDef {
            name: "budget-alert".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s1".to_string(),
                agent: "A".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("do {{task}}".to_string()),
                skill: None,
                output: Some("out".to_string()),
                requires_approval: None,
                r#type: StepType::Execute,
                evaluate: None,
                routes: None,
                workflow: None,
                ..Default::default()
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: Some(1), // would exceed but mode is alert-only
                max_usd_per_task: None,
                alert_at_percent: None,
                on_exceed: Some("alert-only".to_string()),
            }),
            reliability: None,
            eval_gate: None,
        };
        // Should succeed despite exceeding the token limit
        let result = runner.run(&def, "task").await.unwrap();
        assert_eq!(result.steps_completed, 1);
    }

    #[tokio::test]
    async fn evaluator_condition_pass_proceeds() {
        let runner = mock_runner("pass: looks good");
        let def = WorkflowDef {
            name: "eval-pass".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "impl".to_string(),
                    agent: "Coder".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: None,
                    prompt: Some("code {{task}}".to_string()),
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
                    inputs: None,
                    depends_on: Some(vec!["impl".to_string()]),
                    prompt: Some("review {{code}}".to_string()),
                    skill: None,
                    output: Some("verdict".to_string()),
                    requires_approval: None,
                    r#type: StepType::Evaluator,
                    evaluate: Some(EvaluateConfig {
                        condition: Some("{{verdict}} contains 'pass'".to_string()),
                        decision_field: None,
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
                    agent: "Deployer".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: Some(vec!["review".to_string()]),
                    prompt: Some("deploy {{code}}".to_string()),
                    skill: None,
                    output: Some("deployment".to_string()),
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
        let result = runner.run(&def, "build auth").await.unwrap();
        assert!(result.outputs.contains_key("deployment"));
    }

    #[tokio::test]
    async fn evaluator_fail_exceeds_max_retries() {
        let runner = mock_runner("fail: needs work");
        let def = WorkflowDef {
            name: "eval-fail".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "impl".to_string(),
                    agent: "Coder".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: None,
                    prompt: Some("code {{task}}".to_string()),
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
                    inputs: None,
                    depends_on: Some(vec!["impl".to_string()]),
                    prompt: Some("review {{code}}".to_string()),
                    skill: None,
                    output: Some("verdict".to_string()),
                    requires_approval: None,
                    r#type: StepType::Evaluator,
                    evaluate: Some(EvaluateConfig {
                        condition: Some("{{verdict}} contains 'pass'".to_string()),
                        decision_field: None,
                        on_pass: "done".to_string(),
                        on_fail: "impl".to_string(),
                        max_retries: Some(1),
                    }),
                    routes: None,
                    workflow: None,
                    ..Default::default()
                },
                StepDef {
                    id: "done".to_string(),
                    agent: "Deployer".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: Some(vec!["review".to_string()]),
                    prompt: Some("deploy".to_string()),
                    skill: None,
                    output: None,
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
        let err = runner.run(&def, "build auth").await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::MaxRetriesExceeded { .. }
        ));
    }

    #[tokio::test]
    async fn evaluator_fail_rewinds_and_retries_target_step() {
        let runner = sequence_runner(&[
            "draft-v1",
            "fail: needs work",
            "draft-v2",
            "pass: looks good",
            "deploy ok",
        ]);
        let def = WorkflowDef {
            name: "eval-retry".to_string(),
            description: None,
            steps: vec![
                StepDef {
                    id: "impl".to_string(),
                    agent: "Coder".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: None,
                    prompt: Some("code {{task}}".to_string()),
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
                    inputs: None,
                    depends_on: Some(vec!["impl".to_string()]),
                    prompt: Some("review {{code}}".to_string()),
                    skill: None,
                    output: Some("verdict".to_string()),
                    requires_approval: None,
                    r#type: StepType::Evaluator,
                    evaluate: Some(EvaluateConfig {
                        condition: Some("{{verdict}} contains 'pass'".to_string()),
                        decision_field: None,
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
                    agent: "Deployer".to_string(),
                    model: None,
                    inputs: None,
                    depends_on: Some(vec!["review".to_string()]),
                    prompt: Some("deploy {{code}}".to_string()),
                    skill: None,
                    output: Some("deployment".to_string()),
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

        let result = runner.run(&def, "build auth").await.unwrap();
        assert_eq!(
            result.outputs.get("code").map(String::as_str),
            Some("draft-v2")
        );
        assert_eq!(
            result.outputs.get("verdict").map(String::as_str),
            Some("pass: looks good")
        );
        assert_eq!(
            result.outputs.get("deployment").map(String::as_str),
            Some("deploy ok")
        );
    }

    #[tokio::test]
    async fn approval_required_persists_pending_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RunStore::new(dir.path()));
        let run = store
            .create_run("workflow-test", "ship auth", "standalone", Some("mock"))
            .unwrap();
        let runner = mock_runner("approval draft").for_run(store.clone(), run.id.clone());

        let def = WorkflowDef {
            name: "approval-flow".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "approve-me".to_string(),
                agent: "Architect".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("design {{task}}".to_string()),
                skill: None,
                output: Some("plan".to_string()),
                requires_approval: Some(true),
                r#type: StepType::Execute,
                evaluate: None,
                routes: None,
                workflow: None,
                ..Default::default()
            }],
            budget: None,
            reliability: None,
            eval_gate: None,
        };

        let err = runner.run(&def, "ship auth").await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::ApprovalRequired { .. }
        ));

        let state: crate::state::WorkflowRunState = store
            .read_json_artifact(&run.id, "workflow-state.json")
            .unwrap();
        assert_eq!(
            state.status,
            crate::state::WorkflowRunStatus::WaitingApproval
        );
        assert_eq!(
            state
                .pending_approval
                .as_ref()
                .map(|pending| pending.step_id.as_str()),
            Some("approve-me")
        );
        assert_eq!(
            state.steps[0].status,
            crate::state::WorkflowStepStatus::AwaitingApproval
        );
    }

    #[tokio::test]
    async fn router_skips_unselected_branches() {
        // Mock returns "backend" — router should skip ui-work, execute api-work
        let runner = mock_runner("backend");
        let def = WorkflowDef {
            name: "router-test".to_string(),
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
                        RouteConfig {
                            when: Some("frontend".to_string()),
                            goto: "ui-work".to_string(),
                            default: false,
                        },
                        RouteConfig {
                            when: Some("backend".to_string()),
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
        let result = runner.run(&def, "build api").await.unwrap();
        assert!(
            result.outputs.contains_key("api_result"),
            "selected branch should execute"
        );
        assert!(
            !result.outputs.contains_key("ui_result"),
            "unselected branch should be skipped"
        );
    }

    #[tokio::test]
    async fn router_records_shadow_routing_recommendation() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RunStore::new(dir.path()));
        for (route, quality) in [
            ("ui-work", 45.0),
            ("ui-work", 48.0),
            ("api-work", 95.0),
            ("api-work", 91.0),
        ] {
            let run = store
                .create_run("workflow-test", "baseline", "standalone", Some("mock"))
                .unwrap();
            store
                .write_json_artifact(
                    &run.id,
                    "workflow-request.json",
                    &serde_json::json!({
                        "workflow": "router-test",
                        "task": "baseline"
                    }),
                )
                .unwrap();
            store
                .write_json_artifact(
                    &run.id,
                    "workflow-state.json",
                    &serde_json::json!({
                        "workflow": "router-test",
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
            let scorecard = RunScorecard {
                schema_version: 1,
                run_id: run.id.clone(),
                kind: "workflow-test".to_string(),
                workflow: Some("router-test".to_string()),
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
            };
            store
                .write_json_artifact(&run.id, "run-scorecard.json", &scorecard)
                .unwrap();
        }

        let run = store
            .create_run("workflow-test", "build api", "standalone", Some("mock"))
            .unwrap();
        let runner = mock_runner("ui-work").for_run(store.clone(), run.id.clone());
        let def = WorkflowDef {
            name: "router-test".to_string(),
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
                        RouteConfig {
                            when: Some("ui-work".to_string()),
                            goto: "ui-work".to_string(),
                            default: false,
                        },
                        RouteConfig {
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

        let _ = runner.run(&def, "build api").await.unwrap();

        let state: crate::state::WorkflowRunState = store
            .read_json_artifact(&run.id, "workflow-state.json")
            .unwrap();
        assert_eq!(state.routing_recommendations.len(), 1);
        let recommendation = &state.routing_recommendations[0];
        assert_eq!(recommendation.step_id, "classify");
        assert_eq!(recommendation.current_route, "ui-work");
        assert_eq!(recommendation.recommended_route, "api-work");
        assert!(!recommendation.fallback_used);
        assert!(store
            .read_json_artifact_optional::<serde_json::Value>(
                &run.id,
                "routing-recommendations.json"
            )
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn budget_governor_degrades_output_before_abort() {
        let _guard = crate::reliability::test_env_lock();
        std::env::set_var("AGENT007_RELIABILITY_ENABLED", "1");
        std::env::set_var("AGENT007_RELIABILITY_BUDGET_GOVERNOR", "1");
        std::env::set_var("AGENT007_RELIABILITY_DEGRADE_OUTPUT_CHARS", "4");
        std::env::set_var("AGENT007_RELIABILITY_MAX_DEGRADATIONS", "1");

        let runner = mock_runner("abcdefghijklmnopqrstuvwxyz-0123456789");
        let def = WorkflowDef {
            name: "budget-degrade".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "s1".to_string(),
                agent: "A".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("do {{task}}".to_string()),
                skill: None,
                output: Some("out".to_string()),
                requires_approval: None,
                r#type: StepType::Execute,
                evaluate: None,
                routes: None,
                workflow: None,
                ..Default::default()
            }],
            budget: Some(BudgetConfig {
                max_tokens_per_session: Some(6),
                max_usd_per_task: None,
                alert_at_percent: None,
                on_exceed: Some("stop".to_string()),
            }),
            reliability: None,
            eval_gate: None,
        };
        let result = runner.run(&def, "task").await.unwrap();
        let out = result.outputs.get("out").cloned().unwrap_or_default();
        assert!(out.contains("[degraded]"));

        std::env::remove_var("AGENT007_RELIABILITY_ENABLED");
        std::env::remove_var("AGENT007_RELIABILITY_BUDGET_GOVERNOR");
        std::env::remove_var("AGENT007_RELIABILITY_DEGRADE_OUTPUT_CHARS");
        std::env::remove_var("AGENT007_RELIABILITY_MAX_DEGRADATIONS");
    }

    #[tokio::test]
    async fn confidence_escalation_requests_approval_for_low_confidence() {
        let _guard = crate::reliability::test_env_lock();
        std::env::set_var("AGENT007_RELIABILITY_ENABLED", "1");
        std::env::set_var("AGENT007_RELIABILITY_CONFIDENCE_ESCALATION", "1");
        std::env::set_var("AGENT007_AUTO_APPROVE", "0");

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RunStore::new(dir.path()));
        let run = store
            .create_run("workflow-test", "ship auth", "standalone", Some("mock"))
            .unwrap();
        let runner = mock_runner("result\nconfidence: low").for_run(store.clone(), run.id.clone());

        let def = WorkflowDef {
            name: "confidence-escalation".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "draft".to_string(),
                agent: "Architect".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("draft {{task}}".to_string()),
                skill: None,
                output: Some("plan".to_string()),
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
        };

        let err = runner.run(&def, "ship auth").await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::ApprovalRequired { .. }
        ));

        let state: crate::state::WorkflowRunState = store
            .read_json_artifact(&run.id, "workflow-state.json")
            .unwrap();
        assert_eq!(
            state
                .pending_approval
                .as_ref()
                .map(|pending| pending.step_id.as_str()),
            Some("draft")
        );
        assert!(!state.reliability_transitions.is_empty());

        std::env::remove_var("AGENT007_RELIABILITY_ENABLED");
        std::env::remove_var("AGENT007_RELIABILITY_CONFIDENCE_ESCALATION");
        std::env::remove_var("AGENT007_AUTO_APPROVE");
    }

    #[tokio::test]
    async fn guardrails_block_risky_prompt() {
        let _guard = crate::reliability::test_env_lock();
        std::env::set_var("AGENT007_RELIABILITY_ENABLED", "1");
        std::env::set_var("AGENT007_RELIABILITY_GUARDRAILS", "1");

        let runner = mock_runner("ok");
        let def = WorkflowDef {
            name: "guardrails".to_string(),
            description: None,
            steps: vec![StepDef {
                id: "danger".to_string(),
                agent: "Operator".to_string(),
                model: None,
                inputs: None,
                depends_on: None,
                prompt: Some("please drop table users".to_string()),
                skill: None,
                output: Some("result".to_string()),
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
        };

        let err = runner.run(&def, "ship auth").await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::StepFailed { .. }
        ));

        std::env::remove_var("AGENT007_RELIABILITY_ENABLED");
        std::env::remove_var("AGENT007_RELIABILITY_GUARDRAILS");
    }

    #[tokio::test]
    async fn eval_gate_fail_closed_blocks_regressed_release_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RunStore::new(dir.path()));
        for _ in 0..3 {
            seed_baseline_scorecard(&store, "release-gated", 0.0);
        }
        let run = store
            .create_run("workflow-test", "ship auth", "standalone", Some("mock"))
            .unwrap();
        let runner = mock_runner("current release output").for_run(store.clone(), run.id.clone());

        let err = runner
            .run(&release_gated_def(EvalGateMode::FailClosed), "ship auth")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            crate::error::WorkflowError::EvalGateBlocked { .. }
        ));

        let state: crate::state::WorkflowRunState = store
            .read_json_artifact(&run.id, "workflow-state.json")
            .unwrap();
        assert_eq!(state.status, crate::state::WorkflowRunStatus::Failed);
        let decision = state
            .eval_gate_decision
            .expect("eval gate decision missing");
        assert_eq!(
            decision.decision,
            crate::eval_gates::WorkflowEvalGateDecisionKind::Block
        );
        assert!(decision.reason_codes.contains(&"cost-increase".to_string()));
        assert!(store
            .read_json_artifact_optional::<serde_json::Value>(&run.id, "eval-gate-decision.json")
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn eval_gate_fail_open_warns_but_allows_release_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RunStore::new(dir.path()));
        for _ in 0..3 {
            seed_baseline_scorecard(&store, "release-gated", 0.0);
        }
        let run = store
            .create_run("workflow-test", "ship auth", "standalone", Some("mock"))
            .unwrap();
        let runner = mock_runner("current release output").for_run(store.clone(), run.id.clone());

        let result = runner
            .run(&release_gated_def(EvalGateMode::FailOpen), "ship auth")
            .await
            .unwrap();
        assert_eq!(result.steps_completed, 1);
        assert_eq!(
            result.outputs.get("notes").map(String::as_str),
            Some("current release output")
        );

        let state: crate::state::WorkflowRunState = store
            .read_json_artifact(&run.id, "workflow-state.json")
            .unwrap();
        assert_eq!(state.status, crate::state::WorkflowRunStatus::Succeeded);
        let decision = state
            .eval_gate_decision
            .expect("eval gate decision missing");
        assert_eq!(
            decision.decision,
            crate::eval_gates::WorkflowEvalGateDecisionKind::Warn
        );
        assert!(decision.reason_codes.contains(&"cost-increase".to_string()));
    }

    // ── minify_context unit tests ────────────────────────────────────────────

    // ── skill_content_provider builder ─────────────────────────────────────────

    #[test]
    fn with_skill_provider_sets_field() {
        let runner = mock_runner("ok");
        assert!(
            runner.skill_content_provider.is_none(),
            "skill_content_provider should start as None"
        );
        let runner =
            runner.with_skill_provider(Arc::new(agent007_skills::NoOpSkillContentProvider));
        assert!(
            runner.skill_content_provider.is_some(),
            "with_skill_provider should set the field"
        );
    }

    #[test]
    fn with_skill_provider_replaces_existing_provider() {
        use agent007_skills::{Skill, SkillFrontmatter, SkillIndex};
        let first_provider = Arc::new(agent007_skills::NoOpSkillContentProvider);
        let skill = Skill {
            frontmatter: SkillFrontmatter {
                name: "dev-debug".to_string(),
                description: "debug".to_string(),
                trigger: "dev-debug".to_string(),
                model: "claude".to_string(),
                category: "dev".to_string(),
                version: "1.0.0".to_string(),
                tags: vec![],
            },
            template: "debug knowledge".to_string(),
            manifest_path: std::path::PathBuf::from("test.md"),
            entry_path: std::path::PathBuf::from("test.md"),
            skill_dir: std::path::PathBuf::from("."),
        };
        let second_provider = Arc::new(SkillIndex::from_skills(vec![skill]));
        let runner = mock_runner("ok")
            .with_skill_provider(first_provider)
            .with_skill_provider(second_provider);
        let sp = runner.skill_content_provider.as_ref().unwrap();
        // Second provider has content; if it was replaced correctly, lookup succeeds.
        assert!(
            sp.load_content("dev-debug").is_some(),
            "last-set provider should be active"
        );
    }

    // ── WorkerConfig → WorkerSpec mapping ──────────────────────────────────────

    #[test]
    fn worker_config_to_worker_spec_preserves_name_and_skills() {
        use crate::types::{WorkerConfig, WorkerRunMode};
        use agent007_custom_agents::WorkerSpec;

        let wcs = vec![
            WorkerConfig {
                persona: "analyst".to_string(),
                skills: vec!["data-analysis".to_string()],
                run: WorkerRunMode::Parallel,
            },
            WorkerConfig {
                persona: "writer".to_string(),
                skills: vec!["technical-writing".to_string(), "style-guide".to_string()],
                run: WorkerRunMode::Sequential,
            },
        ];

        let specs: Vec<WorkerSpec> = wcs
            .iter()
            .map(|wc| WorkerSpec {
                name: wc.persona.clone(),
                skills: wc.skills.clone(),
                sequential: wc.run == WorkerRunMode::Sequential,
            })
            .collect();

        assert_eq!(specs[0].name, "analyst");
        assert_eq!(specs[0].skills, vec!["data-analysis"]);
        assert!(!specs[0].sequential);
        assert_eq!(specs[1].name, "writer");
        assert!(
            specs[1].sequential,
            "sequential run mode should map to sequential=true"
        );
        assert_eq!(specs[1].skills, vec!["technical-writing", "style-guide"]);
    }

    #[test]
    fn minify_collapses_excess_blank_lines() {
        let input = "line1\n\n\n\n\nline2\n";
        let output = minify_context(input);
        assert!(!output.contains("\n\n\n"), "should collapse 3+ blank lines");
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
    }

    #[test]
    fn minify_strips_trailing_whitespace() {
        let input = "line with trailing   \nanother   \n";
        let output = minify_context(input);
        for line in output.lines() {
            assert_eq!(
                line,
                line.trim_end(),
                "line should have no trailing whitespace"
            );
        }
    }

    #[test]
    fn minify_removes_html_comments() {
        let input = "before\n<!-- this is a comment -->\nafter\n";
        let output = minify_context(input);
        assert!(!output.contains("<!--"));
        assert!(output.contains("before"));
        assert!(output.contains("after"));
    }

    #[test]
    fn minify_collapses_repeated_horizontal_rules() {
        let input = "text\n---\n---\n---\nmore\n";
        let output = minify_context(input);
        let hr_count = output.lines().filter(|l| *l == "---").count();
        assert_eq!(hr_count, 1, "repeated HRs should collapse to one");
    }

    #[test]
    fn minify_skips_json_objects() {
        let input = r#"{"key": "value",  "another":  "val"}"#;
        let output = minify_context(input);
        assert_eq!(output, input, "JSON objects should pass through unchanged");
    }

    #[test]
    fn minify_skips_json_arrays() {
        let input = "[1,  2,  3]";
        let output = minify_context(input);
        assert_eq!(output, input, "JSON arrays should pass through unchanged");
    }

    #[test]
    fn minify_skips_code_fences() {
        let input = "intro\n```\nsome code  here\n```\n";
        let output = minify_context(input);
        assert_eq!(
            output, input,
            "content with code fences should pass through unchanged"
        );
    }

    #[test]
    fn minify_normalizes_multiple_spaces() {
        let input = "word1  word2   word3\n";
        let output = minify_context(input);
        assert!(output.contains("word1 word2 word3"));
    }

    #[test]
    fn minify_preserves_indented_code_blocks() {
        // Any leading whitespace = preserve indentation and internal spaces
        let input_2 = "  two  space  indent\n";
        let output_2 = minify_context(input_2);
        assert!(
            output_2.contains("  two  space  indent"),
            "2-space indent should be untouched"
        );

        let input_4 = "    code   with   spaces\n";
        let output_4 = minify_context(input_4);
        assert!(
            output_4.contains("    code   with   spaces"),
            "4-space indent should be untouched"
        );
    }
}
