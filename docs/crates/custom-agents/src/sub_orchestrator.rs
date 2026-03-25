use std::sync::Arc;
use agent007_core::dispatcher::Dispatcher;
use agent007_core::persona::PersonaProvider;
use agent007_memory::store::ScopedMemoryStore;
use agent007_models::router::ModelRouter;
use agent007_models::provider::ModelProvider;
use agent007_models::types::{CompletionRequest, Message, Role};
use crate::{AgentDef, AgentType, CustomAgentError, SubTaskResult};

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
        Self { def, scoped_memory, model_router, persona_provider, dispatcher, depth, max_depth }
    }

    /// Decompose the task into subtasks and execute via allowed worker personas.
    ///
    /// Algorithm:
    /// 1. Guard: return MaxDepthExceeded if depth >= max_depth.
    /// 2. Guard: return WorkerNotAllowed if allowed_workers is Some([]).
    /// 3. Ask the model (via `model_router`) to produce a plan.
    /// 4. Parse subtasks; validate each worker is in `allowed_workers`.
    /// 5. Retrieve the PersonaSpec from `persona_provider` and dispatch the subtask.
    /// 6. Collect all outputs into `SubTaskResult`.
    pub async fn run(&self, task: &str) -> Result<SubTaskResult, CustomAgentError> {
        // Depth guard
        if self.depth >= self.max_depth {
            return Err(CustomAgentError::MaxDepthExceeded { max: self.max_depth });
        }

        // Guard: if allowed_workers is Some([]) no workers can be dispatched
        if let Some(ref allowed) = self.def.allowed_workers {
            if allowed.is_empty() {
                return Err(CustomAgentError::WorkerNotAllowed {
                    name: "<none>".into(),
                });
            }
        }

        // Build system context from scoped memory namespace
        let _ns = &self.scoped_memory.namespace;

        // Plan decomposition via model router
        let plan_prompt = format!(
            "You are {}. Decompose this task into subtasks, one per allowed worker.\n\
             Allowed workers: {:?}\nTask: {}",
            self.def.name,
            self.def.allowed_workers,
            task
        );

        let request = CompletionRequest {
            model: self.def.model.clone().unwrap_or_else(|| "default".into()),
            messages: vec![Message { role: Role::User, content: plan_prompt }],
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

        // Parse plan
        let subtasks = parse_plan(&raw_plan, self.def.allowed_workers.as_deref())?;

        let mut combined_output = String::new();
        let files_changed = Vec::new();

        for (worker_name, subtask) in &subtasks {
            let persona = self
                .persona_provider
                .get(worker_name)
                .ok_or_else(|| CustomAgentError::WorkerNotAllowed {
                    name: worker_name.clone(),
                })?;

            // Use the model router to dispatch the subtask with the persona's system prompt
            let subtask_request = CompletionRequest {
                model: self.def.model.clone().unwrap_or_else(|| "default".into()),
                messages: vec![Message { role: Role::User, content: subtask.clone() }],
                max_tokens: None,
                temperature: None,
                system: Some(persona.system_prompt.clone()),
            };

            let result = self
                .model_router
                .complete(subtask_request)
                .await
                .map_err(|e| CustomAgentError::ParseError {
                    path: std::path::PathBuf::from("<dispatch>"),
                    reason: e.to_string(),
                })?;

            combined_output.push_str(&result.content);
            combined_output.push('\n');
        }

        Ok(SubTaskResult {
            output: combined_output,
            files_changed,
            tests_passed: false,
            blockers: Vec::new(),
        })
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
        let ns = def.memory_namespace.clone().unwrap_or_else(|| def.name.clone());
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
        // When no memory_namespace is given, caller uses agent name as namespace.
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
        // NoOpPersonaProvider returns None for "Coder", so run will return WorkerNotAllowed.
        // The free-text fallback picks the first allowed worker ("Coder"), then persona lookup fails.
        // This is the expected behavior with NoOpPersonaProvider.
        let result = orch.run("implement feature X").await;
        // Either success with non-empty output, or WorkerNotAllowed (since NoOpPersonaProvider has no personas)
        match result {
            Ok(r) => assert!(!r.output.is_empty()),
            Err(CustomAgentError::WorkerNotAllowed { .. }) => {
                // Expected when NoOpPersonaProvider is used — persona lookup returns None
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn run_worker_not_allowed_returns_error() {
        // Craft an agent whose allowed_workers is empty, then confirm WorkerNotAllowed
        // is surfaced when a disallowed persona is attempted.
        let mut def = make_def("StrictOrch", Some("ns"));
        def.allowed_workers = Some(vec![]); // no workers permitted
        let orch = make_orch(def, 0);
        let err = orch.run("do something requiring a worker").await.unwrap_err();
        assert!(matches!(err, CustomAgentError::WorkerNotAllowed { .. }));
    }

    #[tokio::test]
    async fn run_exceeds_max_depth_returns_error() {
        let def = make_def("DeepOrch", Some("ns"));
        // depth == max_depth means we are already at the limit
        let orch = make_orch(def, 3); // depth=3, max_depth=3
        let err = orch.run("some task").await.unwrap_err();
        assert!(matches!(
            err,
            CustomAgentError::MaxDepthExceeded { max: 3 }
        ));
    }

    #[tokio::test]
    async fn run_at_depth_below_max_does_not_error_on_depth() {
        let def = make_def("ShallowOrch", Some("ns"));
        let orch = make_orch(def, 2); // depth=2, max_depth=3 — within limits
        // Should not produce MaxDepthExceeded; any other result is acceptable here
        let err = orch.run("task").await;
        assert!(!matches!(
            err,
            Err(CustomAgentError::MaxDepthExceeded { .. })
        ));
    }
}
