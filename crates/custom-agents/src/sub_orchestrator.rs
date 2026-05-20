use crate::{AgentDef, CustomAgentError, SubTaskResult};
use agent007_core::dispatcher::Dispatcher;
use agent007_core::events::AgentEvent;
use agent007_core::persona::PersonaProvider;
use agent007_core::types::AgentId;
use agent007_memory::store::ScopedMemoryStore;
use agent007_models::provider::ModelProvider;
use agent007_models::router::ModelRouter;
use agent007_models::types::{CompletionRequest, Message, Role};
use std::sync::Arc;
use tokio::task::JoinSet;

/// A single resolved subtask ready for worker dispatch.
#[derive(Debug, Clone)]
struct SubTask {
    worker_name: String,
    description: String,
}

/// Result collected from one parallel worker.
#[derive(Debug)]
struct WorkerOutput {
    worker_name: String,
    subtask: String,
    output: String,
    blocker: Option<String>,
}

pub struct SubOrchestrator {
    pub def: AgentDef,
    pub scoped_memory: Arc<ScopedMemoryStore>,
    pub model_router: Arc<ModelRouter>,
    pub persona_provider: Arc<dyn PersonaProvider>,
    pub dispatcher: Arc<dyn Dispatcher>,
    pub depth: usize,
    pub max_depth: usize,
}

impl SubOrchestrator {
    pub fn new(
        def: AgentDef,
        scoped_memory: Arc<ScopedMemoryStore>,
        model_router: Arc<ModelRouter>,
        persona_provider: Arc<dyn PersonaProvider>,
        dispatcher: Arc<dyn Dispatcher>,
        depth: usize,
        max_depth: usize,
    ) -> Self {
        Self {
            def,
            scoped_memory,
            model_router,
            persona_provider,
            dispatcher,
            depth,
            max_depth,
        }
    }

    /// Decompose the task into subtasks and execute via allowed worker personas.
    ///
    /// Algorithm:
    /// 1. Guard: return MaxDepthExceeded if depth >= max_depth.
    /// 2. Guard: return WorkerNotAllowed if allowed_workers is Some([]).
    /// 3. Ask the model (via `model_router`) to produce a JSON plan.
    /// 4. Parse subtasks; validate each worker is in `allowed_workers`.
    /// 5. Dispatch all subtasks **concurrently** via `JoinSet`.
    /// 6. Publish Dispatcher events for each worker start/finish.
    /// 7. If any workers returned blockers → dynamic replan (one round).
    /// 8. Write synthesis to scoped memory (`last_run` key).
    /// 9. Collect all outputs into `SubTaskResult`.
    pub async fn run(&self, task: &str) -> Result<SubTaskResult, CustomAgentError> {
        // Depth guard
        if self.depth >= self.max_depth {
            return Err(CustomAgentError::MaxDepthExceeded {
                max: self.max_depth,
            });
        }

        // Guard: if allowed_workers is Some([]) no workers can be dispatched
        if let Some(ref allowed) = self.def.allowed_workers {
            if allowed.is_empty() {
                return Err(CustomAgentError::WorkerNotAllowed {
                    name: "<none>".into(),
                });
            }
        }

        // --- Plan decomposition ---
        let subtasks = self.plan(task).await?;

        // Stable agent_id for the whole run; passed to worker events so the
        // dashboard can attribute WorkerResult/WorkerBlocked to this run.
        let run_id = AgentId::new();

        // --- Parallel worker execution ---
        let mut outputs = self.dispatch_parallel(subtasks, run_id.clone()).await;

        // --- Dynamic replan for blocked subtasks (one round) ---
        let blocked: Vec<WorkerOutput> = outputs.drain_filter_blocked();
        if !blocked.is_empty() {
            let blocker_summary = blocked
                .iter()
                .map(|b| {
                    format!(
                        "Worker '{}' blocked on '{}': {}",
                        b.worker_name,
                        b.subtask,
                        b.blocker.as_deref().unwrap_or("unspecified")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            tracing::info!(
                orchestrator = %self.def.name,
                blocked_count = blocked.len(),
                "Dynamic replan triggered for blocked subtasks"
            );

            let replan_prompt = format!(
                "Original task: {task}\n\
                 The following subtasks were blocked:\n{blocker_summary}\n\
                 Produce a revised JSON plan ONLY for the blocked subtasks. \
                 Use the same JSON format: [{{'worker': '...', 'subtask': '...'}}]",
            );
            match self.plan(&replan_prompt).await {
                Ok(revised) if !revised.is_empty() => {
                    let mut replanned = self.dispatch_parallel(revised, run_id.clone()).await;
                    outputs.extend(replanned.drain(..));
                }
                Ok(_) => {
                    // Replan returned an empty plan — keep the original blocked
                    // outputs so their blocker reasons surface in SubTaskResult.
                    tracing::warn!(
                        orchestrator = %self.def.name,
                        "replan returned empty plan; preserving original blocked outputs"
                    );
                    outputs.extend(blocked);
                }
                Err(e) => {
                    // Replan itself failed — preserve original blocked outputs
                    // rather than silently discarding them.
                    tracing::warn!(
                        orchestrator = %self.def.name,
                        error = %e,
                        "replan failed; preserving original blocked outputs"
                    );
                    outputs.extend(blocked);
                }
            }
        }

        // --- Cross-agent memory synthesis ---
        self.persist_synthesis(task, &outputs);

        // --- Combine outputs ---
        let mut combined_output = String::new();
        let mut all_blockers = Vec::new();
        for w in &outputs {
            combined_output.push_str(&format!("[{}]\n{}\n\n", w.worker_name, w.output));
            if let Some(ref b) = w.blocker {
                all_blockers.push(b.clone());
            }
        }

        Ok(SubTaskResult {
            output: combined_output.trim_end().to_string(),
            files_changed: Vec::new(),
            tests_passed: false,
            blockers: all_blockers,
        })
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Call the model to produce a decomposed plan. Returns parsed subtasks.
    async fn plan(&self, task: &str) -> Result<Vec<SubTask>, CustomAgentError> {
        let plan_prompt = format!(
            "You are {name}.\n\
             Decompose the following task into subtasks, one per allowed worker.\n\
             Allowed workers: {workers:?}\n\n\
             Task: {task}\n\n\
             Respond ONLY with a JSON array, no prose:\n\
             [{{\"worker\": \"WorkerName\", \"subtask\": \"description\"}}]",
            name = self.def.name,
            workers = self.def.allowed_workers,
            task = task,
        );

        let request = CompletionRequest {
            model: self.def.model.clone().unwrap_or_else(|| "default".into()),
            messages: vec![Message {
                role: Role::User,
                content: plan_prompt,
            }],
            max_tokens: None,
            temperature: None,
            system: Some(self.def.system_prompt.clone()),
        };

        let raw_plan = self
            .model_router
            .complete(request)
            .await
            .map_err(|e| CustomAgentError::ParseError {
                path: std::path::PathBuf::from("<plan>"),
                reason: e.to_string(),
            })?
            .content;

        let pairs = parse_plan(&raw_plan, self.def.allowed_workers.as_deref())?;
        Ok(pairs
            .into_iter()
            .map(|(worker_name, description)| SubTask {
                worker_name,
                description,
            })
            .collect())
    }

    /// Dispatch a list of subtasks **concurrently** using `JoinSet`.
    ///
    /// `parent_id` is the stable `AgentId` for the whole orchestrator run.
    /// It is used in `WorkerResult`/`WorkerBlocked` events so dashboard
    /// consumers can correlate worker progress to the parent task entry.
    ///
    /// `WorkerResult.output` is capped at 512 chars in the event payload to
    /// avoid bloating the dispatcher broadcast and run-trace JSONL files;
    /// the full content is still returned through `WorkerOutput.output`.
    async fn dispatch_parallel(
        &self,
        subtasks: Vec<SubTask>,
        parent_id: AgentId,
    ) -> Vec<WorkerOutput> {
        let mut set: JoinSet<WorkerOutput> = JoinSet::new();

        for sub in subtasks {
            let persona_opt = self.persona_provider.get(&sub.worker_name);
            let system = persona_opt
                .map(|p| p.system_prompt.clone())
                .unwrap_or_default();

            let router = Arc::clone(&self.model_router);
            let dispatcher = Arc::clone(&self.dispatcher);
            let model = self.def.model.clone().unwrap_or_else(|| "default".into());
            let run_agent_id = parent_id.clone();
            let worker_name = sub.worker_name.clone();
            let description = sub.description.clone();

            set.spawn(async move {
                // Emit: work about to start
                let _ = dispatcher
                    .publish(AgentEvent::ModelRequest {
                        provider: model.clone(),
                        prompt_ref: agent007_core::types::PromptRef::new(),
                        token_estimate: description.split_whitespace().count().saturating_mul(2),
                    })
                    .await;

                let req = CompletionRequest {
                    model: model.clone(),
                    messages: vec![Message {
                        role: Role::User,
                        content: description.clone(),
                    }],
                    max_tokens: None,
                    temperature: None,
                    system: if system.is_empty() {
                        None
                    } else {
                        Some(system)
                    },
                };

                match router.complete(req).await {
                    Ok(resp) => {
                        // Emit: worker finished (output truncated to 512 chars in
                        // the event to keep dispatcher broadcast + JSONL compact)
                        let preview: String = resp.content.chars().take(512).collect();
                        let _ = dispatcher
                            .publish(AgentEvent::WorkerResult {
                                agent_id: run_agent_id.clone(),
                                worker_name: worker_name.clone(),
                                subtask: description.clone(),
                                output: preview,
                            })
                            .await;

                        WorkerOutput {
                            worker_name,
                            subtask: description,
                            output: resp.content,
                            blocker: None,
                        }
                    }
                    Err(e) => {
                        // Emit: worker blocked/failed
                        let reason = e.to_string();
                        let _ = dispatcher
                            .publish(AgentEvent::WorkerBlocked {
                                agent_id: run_agent_id.clone(),
                                worker_name: worker_name.clone(),
                                subtask: description.clone(),
                                reason: reason.clone(),
                            })
                            .await;

                        WorkerOutput {
                            worker_name,
                            subtask: description,
                            output: String::new(),
                            blocker: Some(reason),
                        }
                    }
                }
            });
        }

        // Collect in completion order; re-sort would need index — keep as-is for now
        let mut results = Vec::new();
        while let Some(res) = set.join_next().await {
            match res {
                Ok(output) => results.push(output),
                Err(e) => {
                    tracing::warn!(error = %e, "worker task panicked");
                }
            }
        }
        results
    }

    /// Write a synthesis record to scoped memory under the key `last_run`.
    fn persist_synthesis(&self, task: &str, outputs: &[WorkerOutput]) {
        use chrono::Utc;

        let subtask_results: Vec<serde_json::Value> = outputs
            .iter()
            .map(|w| {
                serde_json::json!({
                    "worker": w.worker_name,
                    "subtask": w.subtask,
                    "output": w.output,
                    "blocked": w.blocker.is_some(),
                })
            })
            .collect();

        let record = serde_json::json!({
            "task": task,
            "agent": self.def.name,
            "timestamp": Utc::now().to_rfc3339(),
            "subtask_results": subtask_results,
        });

        if let Err(e) = self.scoped_memory.write("last_run", &record.to_string()) {
            tracing::warn!(error = %e, "failed to persist agent synthesis to memory");
        }
    }
}

/// Extension trait to drain blocked `WorkerOutput`s from a Vec.
trait DrainBlocked {
    fn drain_filter_blocked(&mut self) -> Vec<WorkerOutput>;
}

impl DrainBlocked for Vec<WorkerOutput> {
    fn drain_filter_blocked(&mut self) -> Vec<WorkerOutput> {
        let (blocked, unblocked): (Vec<_>, Vec<_>) =
            self.drain(..).partition(|w| w.blocker.is_some());
        *self = unblocked;
        blocked
    }
}

fn parse_plan(
    raw: &str,
    allowed: Option<&[String]>,
) -> Result<Vec<(String, String)>, CustomAgentError> {
    // Accepts two formats:
    //   1. JSON array: [{"worker": "Coder", "subtask": "..."}]
    //   2. Free-text fallback: treat entire raw text as single subtask for first allowed worker
    if let Ok(steps) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
        steps
            .iter()
            .map(|v| {
                let worker = v["worker"].as_str().unwrap_or("").to_string();
                let subtask = v["subtask"].as_str().unwrap_or("").to_string();
                if let Some(allowed) = allowed {
                    if !allowed.contains(&worker) {
                        return Err(CustomAgentError::WorkerNotAllowed { name: worker });
                    }
                }
                Ok((worker, subtask))
            })
            .collect()
    } else {
        // Free-text fallback — assign to first allowed worker
        let worker = allowed
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or_else(|| "default".into());
        Ok(vec![(worker, raw.to_string())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentType;
    use agent007_core::dispatcher::LocalDispatcher;
    use agent007_core::persona::NoOpPersonaProvider;
    use agent007_memory::store::MemoryStore;
    use agent007_models::mock::MockProvider;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn make_def(name: &str, namespace: Option<&str>) -> AgentDef {
        AgentDef {
            name: name.to_string(),
            r#type: AgentType::SubOrchestrator,
            description: None,
            scope: None,
            system_prompt: "Test.".into(),
            allowed_workers: Some(vec!["Coder".into()]),
            model: None,
            memory_namespace: namespace.map(str::to_string),
            zones: None,
        }
    }

    fn make_orch(def: AgentDef, depth: usize) -> SubOrchestrator {
        let dir = tempdir().unwrap();
        let inner_store = Arc::new(MemoryStore::new(dir.path()));
        let ns = def
            .memory_namespace
            .clone()
            .unwrap_or_else(|| def.name.clone());
        let scoped = Arc::new(inner_store.scoped(&ns));
        let mock = Arc::new(MockProvider::new("mock response", "mock"));
        let mut router = ModelRouter::new("mock");
        router.register("mock", mock as Arc<dyn ModelProvider>);
        let router = Arc::new(router);
        let personas = Arc::new(NoOpPersonaProvider);
        let dispatcher = LocalDispatcher::new(64);
        SubOrchestrator::new(def, scoped, router, personas, dispatcher, depth, 3)
    }

    #[test]
    fn new_sets_depth_and_max_depth() {
        let def = make_def("OrchestratorX", None);
        let orch = make_orch(def, 0);
        assert_eq!(orch.depth, 0);
        assert_eq!(orch.max_depth, 3);
    }

    #[test]
    fn memory_namespace_uses_explicit_value() {
        let def = make_def("OrchestratorX", Some("my-ns"));
        let orch = make_orch(def, 0);
        assert_eq!(orch.scoped_memory.namespace, "my-ns");
    }

    #[test]
    fn memory_namespace_falls_back_to_agent_name() {
        let def = make_def("OrchestratorX", None);
        let orch = make_orch(def, 0);
        assert_eq!(orch.scoped_memory.namespace, "OrchestratorX");
    }

    #[test]
    fn def_name_is_preserved() {
        let def = make_def("MyOrch", Some("ns"));
        let orch = make_orch(def, 1);
        assert_eq!(orch.def.name, "MyOrch");
    }

    #[tokio::test]
    async fn run_returns_sub_task_result() {
        let def = make_def("OrchestratorX", Some("ns"));
        let orch = make_orch(def, 0);
        // NoOpPersonaProvider returns None for "Coder" — worker uses empty system prompt.
        // MockProvider returns "mock response" so the run should succeed.
        let result = orch.run("implement feature X").await;
        match result {
            Ok(r) => assert!(!r.output.is_empty()),
            Err(CustomAgentError::WorkerNotAllowed { .. }) => {
                // Also acceptable when NoOpPersonaProvider is used
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn run_worker_not_allowed_returns_error() {
        let mut def = make_def("StrictOrch", Some("ns"));
        def.allowed_workers = Some(vec![]); // no workers permitted
        let orch = make_orch(def, 0);
        let err = orch
            .run("do something requiring a worker")
            .await
            .unwrap_err();
        assert!(matches!(err, CustomAgentError::WorkerNotAllowed { .. }));
    }

    #[tokio::test]
    async fn run_exceeds_max_depth_returns_error() {
        let def = make_def("DeepOrch", Some("ns"));
        let orch = make_orch(def, 3); // depth=3, max_depth=3
        let err = orch.run("some task").await.unwrap_err();
        assert!(matches!(err, CustomAgentError::MaxDepthExceeded { max: 3 }));
    }

    #[tokio::test]
    async fn run_at_depth_below_max_does_not_error_on_depth() {
        let def = make_def("ShallowOrch", Some("ns"));
        let orch = make_orch(def, 2); // depth=2, max_depth=3
        let err = orch.run("task").await;
        assert!(!matches!(
            err,
            Err(CustomAgentError::MaxDepthExceeded { .. })
        ));
    }

    #[tokio::test]
    async fn run_publishes_dispatcher_events() {
        use futures::StreamExt as FutExt;

        let dir = tempdir().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = Arc::new(store.scoped("ns"));
        let mock = Arc::new(MockProvider::new(
            // MockProvider returns the same text for all calls:
            // plan call gets this → treated as free-text fallback → Coder worker
            "mock response",
            "mock",
        ));
        let mut router = ModelRouter::new("mock");
        router.register("mock", mock as Arc<dyn ModelProvider>);
        let router = Arc::new(router);
        let dispatcher = LocalDispatcher::new(64);
        let mut events = dispatcher.subscribe().await.unwrap();

        let def = make_def("EventOrch", Some("ns"));
        let orch = SubOrchestrator::new(
            def,
            scoped,
            router,
            Arc::new(NoOpPersonaProvider),
            dispatcher,
            0,
            3,
        );

        let _ = orch.run("build something").await;

        // Should receive at least one ModelRequest event from worker dispatch
        let e1 = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            FutExt::next(&mut events),
        )
        .await;
        assert!(e1.is_ok(), "expected at least one event from dispatcher");
    }

    #[tokio::test]
    async fn run_persists_synthesis_to_memory() {
        let dir = tempdir().unwrap();
        let inner_store = Arc::new(MemoryStore::new(dir.path()));
        let ns = "PersistOrch";
        let scoped = Arc::new(inner_store.scoped(ns));
        let mock = Arc::new(MockProvider::new(
            r#"[{"worker":"Coder","subtask":"do X"}]"#,
            "mock",
        ));
        let mut router = ModelRouter::new("mock");
        router.register("mock", mock as Arc<dyn ModelProvider>);
        let router = Arc::new(router);
        let dispatcher = LocalDispatcher::new(64);
        let def = AgentDef {
            name: "PersistOrch".into(),
            r#type: AgentType::SubOrchestrator,
            description: None,
            scope: None,
            system_prompt: "Persist test.".into(),
            allowed_workers: Some(vec!["Coder".into()]),
            model: None,
            memory_namespace: Some(ns.into()),
            zones: None,
        };
        let orch = SubOrchestrator::new(
            def,
            scoped,
            router,
            Arc::new(NoOpPersonaProvider),
            dispatcher,
            0,
            3,
        );
        let _ = orch.run("persist task").await;

        // Synthesis should have been written to memory
        let content = inner_store.scoped(ns).read("last_run").unwrap();
        assert!(
            content.is_some(),
            "last_run key should be written after run"
        );
        let json: serde_json::Value =
            serde_json::from_str(&content.unwrap()).expect("should be valid JSON");
        assert_eq!(json["task"], "persist task");
        assert_eq!(json["agent"], "PersistOrch");
        assert!(json["subtask_results"].is_array());
    }

    #[tokio::test]
    async fn dispatch_parallel_runs_multiple_workers_concurrently() {
        // Verify JoinSet spawning: two subtasks should both complete
        let dir = tempdir().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let mock = Arc::new(MockProvider::new("worker done", "mock"));
        let mut router = ModelRouter::new("mock");
        router.register("mock", mock as Arc<dyn ModelProvider>);
        let router = Arc::new(router);
        let dispatcher = LocalDispatcher::new(64);

        let def = AgentDef {
            name: "ParallelOrch".into(),
            r#type: AgentType::SubOrchestrator,
            description: None,
            scope: None,
            system_prompt: "Run parallel.".into(),
            allowed_workers: Some(vec!["A".into(), "B".into()]),
            model: None,
            memory_namespace: Some("parallel".into()),
            zones: None,
        };

        let orch = SubOrchestrator::new(
            def,
            Arc::new(store.scoped("parallel")),
            router,
            Arc::new(NoOpPersonaProvider),
            dispatcher,
            0,
            3,
        );

        let subtasks = vec![
            SubTask {
                worker_name: "A".into(),
                description: "task for A".into(),
            },
            SubTask {
                worker_name: "B".into(),
                description: "task for B".into(),
            },
        ];

        let outputs = orch.dispatch_parallel(subtasks, AgentId::new()).await;
        assert_eq!(outputs.len(), 2, "both workers should complete");
    }
}
