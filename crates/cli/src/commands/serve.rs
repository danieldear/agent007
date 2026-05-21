use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};

use anyhow::Result;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, Content, Implementation, InitializeRequestParams,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
    transport::io::stdio,
    ServerHandler, ServiceExt,
};
use serde_json::Map;

use super::run::{
    agent007_global_home, agent007_home, agent007_project_home, agent007_write_home, build_stack,
    build_stack_for_web, provider_readiness_response, runtime_mode_label, selected_runtime_model,
    selected_runtime_provider, standalone_mode_available,
};
use super::skill::SkillSummary;
use super::slash_commands::sync_claude_slash_commands_for_home;
use crate::config::Config;

use agent007_core::dispatcher::LocalDispatcher;
use agent007_core::events::AgentEvent;
use agent007_core::persona::PersonaSpec;
use agent007_core::types::PromptRef;
use agent007_hooks::{HookConfig, HookEvent, HookExecutor};
use agent007_learning::LearningDispatcher;

/// MCP server that exposes agent007 tools to Claude Code (or any MCP client).
pub struct Agent007Server {
    config: Arc<Config>,
    dispatcher: Option<Arc<LocalDispatcher>>,
    learning_dispatcher: Option<Arc<LearningDispatcher>>,
    hook_executor: Option<Arc<HookExecutor>>,
}

impl Agent007Server {
    pub fn new(config: Arc<Config>) -> Self {
        let hook_executor = load_hook_executor();
        Self {
            config,
            dispatcher: None,
            learning_dispatcher: None,
            hook_executor,
        }
    }

    pub fn with_dispatchers(
        mut self,
        dispatcher: Arc<LocalDispatcher>,
        learning_dispatcher: Arc<LearningDispatcher>,
    ) -> Self {
        self.dispatcher = Some(dispatcher);
        self.learning_dispatcher = Some(learning_dispatcher);
        self
    }

    fn fire_hook(&self, event: &HookEvent) {
        if let Some(exec) = &self.hook_executor {
            if let Err(e) = exec.fire(event) {
                tracing::warn!(error = %e, "hook fire failed");
            }
        }
    }

    async fn publish_task_assigned(
        &self,
        agent_id: &agent007_core::types::AgentId,
        task: &agent007_core::Task,
    ) {
        if let Some(d) = &self.dispatcher {
            use agent007_core::dispatcher::Dispatcher;
            let _ = d
                .publish(agent007_core::AgentEvent::TaskAssigned {
                    agent_id: agent_id.clone(),
                    task: task.clone(),
                })
                .await;
        }
    }

    async fn publish_task_completed(
        &self,
        agent_id: &agent007_core::types::AgentId,
        task_id: uuid::Uuid,
        output: &str,
    ) {
        if let Some(d) = &self.dispatcher {
            use agent007_core::dispatcher::Dispatcher;
            let _ = d
                .publish(agent007_core::AgentEvent::TaskCompleted {
                    agent_id: agent_id.clone(),
                    result: agent007_core::TaskResult::success(
                        task_id,
                        output.chars().take(200).collect(),
                    ),
                    skill_name: None,
                    model: None,
                })
                .await;
        }
        self.fire_hook(&HookEvent::PostTaskComplete);
    }

    async fn publish_model_request(&self, token_estimate: usize) {
        if let Some(d) = &self.dispatcher {
            use agent007_core::dispatcher::Dispatcher;
            let _ = d
                .publish(agent007_core::AgentEvent::ModelRequest {
                    provider: selected_runtime_provider(&self.config)
                        .unwrap_or_else(|| "hosted-mcp".to_string()),
                    prompt_ref: agent007_core::types::PromptRef::new(),
                    token_estimate,
                })
                .await;
        }
    }

    fn base_tool_defs() -> Vec<Tool> {
        vec![
            // ── existing 5 tools ───────────────────────────────────────────
            tool(
                "agent007_help",
                "Show the agent007 MCP catalog with exact invokable tool names, grouped by core tools, skills, and workflows.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "topic": {
                            "type": "string",
                            "description": "Optional topic filter: overview, skills, workflows, or tools",
                            "enum": ["overview", "skills", "workflows", "tools"]
                        }
                    }
                }),
            ),
            tool(
                "agent007_run",
                "Run a task with the agent007 orchestration stack (ModelRouter, RAG memory, \
                 skills, hooks, MCP tools, learning). Submits the task and returns confirmation.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "The task description to execute"
                        }
                    },
                    "required": ["task"]
                }),
            ),
            tool(
                "agent007_dispatch",
                "Dispatch a simple command-style agent007 request (slash-like UX) to the right tool. \
                 Supports patterns like '$agent007 wf tdd ...', '$agent007 skill /brainstorm ...', \
                 '$agent007 run ...', '/agent007 ...', or '@agent007 ...'.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Command-style request to dispatch"
                        }
                    },
                    "required": ["command"]
                }),
            ),
            tool(
                "agent007_skill_list",
                "List all available skills loaded from .agent007/skills/ (project) and ~/.agent007/skills/ (global)",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),
            tool(
                "agent007_skill_run",
                "Run a specific agent007 skill by its trigger (e.g. /review-pr)",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "trigger": {
                            "type": "string",
                            "description": "Skill trigger, e.g. /review-pr"
                        },
                        "args": {
                            "type": "string",
                            "description": "Arguments to pass to the skill template",
                            "default": ""
                        }
                    },
                    "required": ["trigger"]
                }),
            ),
            tool(
                "agent007_persona_list",
                "List all available agent007 personas (built-in + user overrides from ~/.agent007/personas/). \
                 Returns name, preferred model, and description for each persona.",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),
            tool(
                "agent007_persona_show",
                "Show full details (including system prompt and allowed tools) for a named agent007 persona.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Exact persona name, e.g. Researcher"
                        }
                    },
                    "required": ["name"]
                }),
            ),

            // ── new tools ──────────────────────────────────────────────────

            // 1. Memory read
            tool(
                "agent007_memory_read",
                "Read a value from the agent007 memory store by scope and key.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "scope": {
                            "type": "string",
                            "description": "Memory scope: 'global', 'user', 'project', or any custom namespace"
                        },
                        "key": {
                            "type": "string",
                            "description": "Key to read"
                        }
                    },
                    "required": ["scope", "key"]
                }),
            ),

            // 2. Memory write
            tool(
                "agent007_memory_write",
                "Write a key/value pair to the agent007 memory store.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "scope": {
                            "type": "string",
                            "description": "Memory scope: 'global', 'user', 'project', or any custom namespace"
                        },
                        "key": {
                            "type": "string",
                            "description": "Key to write"
                        },
                        "value": {
                            "type": "string",
                            "description": "Value to store"
                        }
                    },
                    "required": ["scope", "key", "value"]
                }),
            ),

            // 3. Memory list
            tool(
                "agent007_memory_list",
                "List all keys stored in a memory scope.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "scope": {
                            "type": "string",
                            "description": "Memory scope: 'global', 'user', 'project', or any custom namespace"
                        }
                    },
                    "required": ["scope"]
                }),
            ),

            // 4. Record actual tokens (call this after you finish LLM work so the dashboard shows real counts)
            tool(
                "agent007_record_tokens",
                "Record the actual token usage for a run. Call this after completing LLM work in hosted-MCP mode \
                 so the dashboard shows accurate token counts instead of estimates. \
                 Pass the run_id from the original skill/task response, the actual total tokens used, \
                 the model name, and optionally the output text so it gets saved to project memory \
                 for future context reuse.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "run_id": {
                            "type": "string",
                            "description": "The run_id returned by the original skill or task call"
                        },
                        "tokens": {
                            "type": "integer",
                            "description": "Actual total tokens used (input + output)"
                        },
                        "model": {
                            "type": "string",
                            "description": "Model name that was used, e.g. 'claude-sonnet-4-6'"
                        },
                        "output": {
                            "type": "string",
                            "description": "Optional: the final output/result text from the skill or task. \
                                           When provided, it is saved to project memory so future skill \
                                           invocations can retrieve it as context, reducing repeated analysis."
                        }
                    },
                    "required": ["run_id", "tokens", "model"]
                }),
            ),

            // 5. Workflow list
            tool(
                "agent007_workflow_list",
                "List workflow YAML files in ~/.agent007/workflows/.",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),

            // 5. Workflow run
            tool(
                "agent007_workflow_run",
                "Run a named workflow from ~/.agent007/workflows/<name>.yaml with a task description. \
                 Steps without dependencies run in parallel; steps with depends_on run after their \
                 predecessors complete. Returns a JSON object with each step's output.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Workflow name (filename without .yaml extension)"
                        },
                        "task": {
                            "type": "string",
                            "description": "The task description injected as {{task}} in every step prompt"
                        }
                    },
                    "required": ["name", "task"]
                }),
            ),

            tool(
                "agent007_workflow_resume",
                "Resume a persisted workflow run from a prior agent007 session ID. \
                 The source session must contain workflow-request.json and workflow-state.json artifacts.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session": {
                            "type": "string",
                            "description": "Prior workflow session ID from agent007_run_history"
                        }
                    },
                    "required": ["session"]
                }),
            ),

            tool(
                "agent007_workflow_approve",
                "Record an approval decision for a persisted workflow session. \
                 Use this after agent007_workflow_run or agent007_workflow_resume returns an approval-required error.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session": {
                            "type": "string",
                            "description": "Workflow session ID that is waiting for approval"
                        },
                        "step": {
                            "type": "string",
                            "description": "Optional step ID. Defaults to the current pending approval step."
                        },
                        "decision": {
                            "type": "string",
                            "description": "approve, deny, or edit"
                        },
                        "content": {
                            "type": "string",
                            "description": "Edited content to use when decision=edit"
                        }
                    },
                    "required": ["session", "decision"]
                }),
            ),

            tool(
                "agent007_workflow_start",
                "Start a hosted MCP workflow session. agent007 keeps the workflow state, and the host LLM executes returned steps and submits outputs back.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Workflow name (filename without extension)"
                        },
                        "task": {
                            "type": "string",
                            "description": "Task description injected as {{task}}"
                        }
                    },
                    "required": ["name", "task"]
                }),
            ),

            tool(
                "agent007_workflow_next",
                "Fetch and lease the next ready hosted workflow steps for a session. Use after start or after submitting prior step outputs.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session": {
                            "type": "string",
                            "description": "Hosted workflow session ID"
                        }
                    },
                    "required": ["session"]
                }),
            ),

            tool(
                "agent007_workflow_submit_step",
                "Submit the output for a hosted workflow step. agent007 applies approvals, retries, routing, and returns the updated workflow status and next ready steps.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session": {
                            "type": "string",
                            "description": "Hosted workflow session ID"
                        },
                        "step": {
                            "type": "string",
                            "description": "Step ID being completed"
                        },
                        "output": {
                            "type": "string",
                            "description": "The host-generated output for the step"
                        },
                        "tokens": {
                            "type": "integer",
                            "description": "Actual total tokens used (input+output) for this step. Provide if available for accurate dashboard metrics."
                        }
                    },
                    "required": ["session", "step", "output"]
                }),
            ),

            tool(
                "agent007_workflow_status",
                "Inspect the current state of a hosted workflow session without leasing new steps.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session": {
                            "type": "string",
                            "description": "Hosted workflow session ID"
                        }
                    },
                    "required": ["session"]
                }),
            ),

            tool(
                "agent007_workflow_get_output",
                "Fetch a named step output from a hosted workflow session. Use this inside a step agent to retrieve prior outputs on demand, avoiding token bloat in the orchestrating context.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session": {
                            "type": "string",
                            "description": "Hosted workflow session ID"
                        },
                        "key": {
                            "type": "string",
                            "description": "The output key name (e.g. 'ranging_report', 'code')"
                        }
                    },
                    "required": ["session", "key"]
                }),
            ),

            tool(
                "agent007_workflow_heartbeat",
                "Report progress from inside a running workflow step. Keeps the step alive in the watchdog and surfaces a progress hint on the dashboard.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session": {
                            "type": "string",
                            "description": "Hosted workflow session ID"
                        },
                        "step": {
                            "type": "string",
                            "description": "Step ID sending the heartbeat"
                        },
                        "hint": {
                            "type": "string",
                            "description": "Short description of current progress (e.g. 'analyzing phase 2 of 5')"
                        }
                    },
                    "required": ["session", "step"]
                }),
            ),

            // 6. Git status
            tool(
                "agent007_git_status",
                "Run git status in the current working directory and return the output.",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),

            // 7. Git diff
            tool(
                "agent007_git_diff",
                "Run git diff (staged + unstaged) in the current working directory.",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),

            // 8. Git log
            tool(
                "agent007_git_log",
                "Show the last N git commits in the current working directory.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "n": {
                            "type": "number",
                            "description": "Number of commits to show (default: 10)"
                        }
                    }
                }),
            ),

            // 9. Git commit
            tool(
                "agent007_git_commit",
                "Create a git commit with the provided message in the current working directory.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "Commit message"
                        }
                    },
                    "required": ["message"]
                }),
            ),

            // 10. Persona switch
            tool(
                "agent007_persona_switch",
                "Switch the active agent persona. Stores the selection in memory under key 'active_persona'.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Persona name to activate, e.g. Researcher"
                        }
                    },
                    "required": ["name"]
                }),
            ),

            // 11. Zone check
            tool(
                "agent007_zone_check",
                "Check whether an operation on a path is allowed by the configured zone rules.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path to check"
                        },
                        "operation": {
                            "type": "string",
                            "description": "Operation to check: 'read', 'write', or 'execute'"
                        }
                    },
                    "required": ["path", "operation"]
                }),
            ),

            // 12. Task submit
            tool(
                "agent007_task_submit",
                "Submit a task to the agent007 orchestrator and return the task ID.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "Task description"
                        },
                        "persona": {
                            "type": "string",
                            "description": "Optional persona name to use for this task"
                        }
                    },
                    "required": ["task"]
                }),
            ),

            // 13. Skill create
            tool(
                "agent007_skill_create",
                "Create a new skill markdown file in ~/.agent007/skills/.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Skill name"
                        },
                        "trigger": {
                            "type": "string",
                            "description": "Skill trigger string, e.g. /my-skill"
                        },
                        "description": {
                            "type": "string",
                            "description": "Short description of what this skill does"
                        },
                        "template": {
                            "type": "string",
                            "description": "Skill prompt template body"
                        },
                        "model": {
                            "type": "string",
                            "description": "Model or provider to use (default: configured provider)"
                        }
                    },
                    "required": ["name", "trigger", "description", "template"]
                }),
            ),

            // 14. Config show
            tool(
                "agent007_config_show",
                "Show the current agent007 configuration as TOML.",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),

            // 15. Health
            tool(
                "agent007_health",
                "Show health status: memory dir, skills count, personas count, zones configured.",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),

            // 16. Workflow plan (delegate mode)
            tool(
                "agent007_workflow_plan",
                "Return a workflow execution plan with structured step prompts, personas, and \
                 dependency graph — WITHOUT executing. The host LLM drives execution step by step. \
                 Each step includes: id, agent persona (with system prompt), rendered prompt, \
                 dependencies, and output variable name.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Workflow name (filename without .yaml extension)"
                        },
                        "task": {
                            "type": "string",
                            "description": "The task description injected as {{task}} in every step prompt"
                        }
                    },
                    "required": ["name", "task"]
                }),
            ),

            // 17. Agent create (wizard mode)
            tool(
                "agent007_agent_create",
                "Create or browse agent personas. Use action='catalog' to get all available \
                 archetypes with defaults. Use action='save' with full spec to create a new \
                 persona or customize an existing archetype. Saved to .agent007/personas/.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "Action: 'catalog' (list archetypes) or 'save' (create/update persona)",
                            "enum": ["catalog", "save"]
                        },
                        "name": {
                            "type": "string",
                            "description": "Persona name (required for save)"
                        },
                        "description": {
                            "type": "string",
                            "description": "Short description (required for save)"
                        },
                        "system_prompt": {
                            "type": "string",
                            "description": "Full system prompt (required for save)"
                        },
                        "preferred_model": {
                            "type": "string",
                            "description": "Model or provider to use (default: configured provider)"
                        },
                        "allowed_tools": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Tools this agent can use (e.g. bash, file_read, file_write)"
                        },
                        "skills": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Default skill triggers injected for this persona"
                        },
                        "agent_type": {
                            "type": "string",
                            "description": "'worker' or 'orchestrator'"
                        },
                        "allowed_workers": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Worker persona names allowed when this persona is an orchestrator"
                        },
                        "memory_namespace": {
                            "type": "string",
                            "description": "Optional memory namespace for this persona"
                        }
                    },
                    "required": ["action"]
                }),
            ),

            // 18. Skill wizard
            tool(
                "agent007_skill_wizard",
                "Create skills with templates. Use action='templates' to get available skill \
                 templates with defaults. Use action='save' with full spec to create a new skill. \
                 Saved to .agent007/skills/.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "Action: 'templates' (list templates) or 'save' (create skill)",
                            "enum": ["templates", "save"]
                        },
                        "name": {
                            "type": "string",
                            "description": "Skill name (required for save)"
                        },
                        "trigger": {
                            "type": "string",
                            "description": "Trigger string, e.g. /my-skill (required for save)"
                        },
                        "description": {
                            "type": "string",
                            "description": "Short description (required for save)"
                        },
                        "template": {
                            "type": "string",
                            "description": "Prompt template body with {{args}} placeholder (required for save)"
                        },
                        "model": {
                            "type": "string",
                            "description": "Model or provider to use (default: configured provider)"
                        }
                    },
                    "required": ["action"]
                }),
            ),

            // 19. Downstream MCP tool list
            tool(
                "agent007_mcp_tools_list",
                "List tools exposed by downstream MCP servers configured under [mcp.servers] in agent007 config.",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),

            // 20. Downstream MCP tool call
            tool(
                "agent007_mcp_tool_call",
                "Call a tool exposed by a downstream MCP server configured in agent007.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Exact downstream MCP tool name"
                        },
                        "args": {
                            "type": "object",
                            "description": "JSON object arguments passed to the downstream tool",
                            "default": {}
                        }
                    },
                    "required": ["name"]
                }),
            ),

            // 21. Run history
            tool(
                "agent007_run_history",
                "List the most recent recorded agent007 runs from ~/.agent007/sessions/.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "limit": {
                            "type": "number",
                            "description": "Maximum number of runs to return (default: 10)"
                        }
                    }
                }),
            ),

            // 22. Run show
            tool(
                "agent007_run_show",
                "Show metadata and the captured event trace for a recorded run ID.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Run ID from agent007_run_history"
                        }
                    },
                    "required": ["id"]
                }),
            ),

            tool(
                "agent007_compact_output",
                "Compact noisy command output into a high-signal summary and record both raw and compact artifacts.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Original command, e.g. cargo test or git diff"
                        },
                        "output": {
                            "type": "string",
                            "description": "Raw command output to compact"
                        },
                        "level": {
                            "type": "string",
                            "description": "Compaction level: full, compact, or aggressive"
                        }
                    },
                    "required": ["command", "output"]
                }),
            ),

            tool(
                "agent007_context_compile",
                "Compile a task-scoped context bundle: repo brain, relevant files, memory notes, recent runs, and a rendered compact context block.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "Current task the host LLM is trying to solve"
                        },
                        "max_files": {
                            "type": "number",
                            "description": "Maximum number of relevant files to include"
                        },
                        "max_memory_notes": {
                            "type": "number",
                            "description": "Maximum number of project memory notes to include"
                        },
                        "max_prompt_tokens": {
                            "type": "number",
                            "description": "Prompt budget ceiling used for policy recommendations"
                        },
                        "reserve_tokens": {
                            "type": "number",
                            "description": "Reserved reasoning buffer inside the prompt budget"
                        },
                        "max_response_tokens": {
                            "type": "number",
                            "description": "Expected response token budget"
                        }
                    },
                    "required": ["task"]
                }),
            ),

            tool(
                "agent007_repo_brain_refresh",
                "Distill the current repository into a persistent repo brain summary and save it into project memory.",
                serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            ),

            tool(
                "agent007_agent_run",
                "Run a named custom agent (sub-orchestrator or worker) on a task. \
                 The agent decomposes the task into subtasks, dispatches them to worker \
                 personas in parallel, synthesises a combined result, and persists a \
                 last_run summary to its memory namespace. \
                 Use the `agent007 agent list` CLI command to see available agents.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name of the agent to run (must match a TOML file in ~/.agent007/agents/)"
                        },
                        "task": {
                            "type": "string",
                            "description": "Task description for the agent to execute"
                        }
                    },
                    "required": ["name", "task"]
                }),
            ),

            tool(
                "agent007_budget_estimate",
                "Estimate prompt budget pressure for a task or text block and recommend whether to use full, compact, or aggressive context.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "Optional task description"
                        },
                        "text": {
                            "type": "string",
                            "description": "Text to estimate for prompt budget usage"
                        },
                        "max_prompt_tokens": {
                            "type": "number",
                            "description": "Prompt budget ceiling"
                        },
                        "reserve_tokens": {
                            "type": "number",
                            "description": "Reserved reasoning buffer"
                        },
                        "max_response_tokens": {
                            "type": "number",
                            "description": "Expected response token budget"
                        }
                    },
                    "propertiesOrder": ["task", "text", "max_prompt_tokens", "reserve_tokens", "max_response_tokens"]
                }),
            ),

            // ETR — Embedded Tool Runtime
            tool(
                "agent007_etr_call",
                "Call an ETR (Embedded Tool Runtime) tool. L1 tools run natively in Rust with < 1 ms \
                 latency and zero LLM tokens. Available tools: etr.grep, etr.json_extract, etr.csv_slice, \
                 etr.glob, etr.file_stat, etr.math, etr.diff, etr.list. Use agent007_etr_list to discover \
                 schemas before calling.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "tool": {
                            "type": "string",
                            "description": "Tool name (e.g. 'etr.grep', 'etr.csv_slice', 'etr.list')"
                        },
                        "input": {
                            "type": "object",
                            "description": "Tool input as a JSON object — schema varies by tool. Use etr.list to see schemas."
                        },
                        "compact": {
                            "type": "boolean",
                            "description": "If true (default), large outputs are compacted. Set false to get full output (L1/L2 only).",
                            "default": true
                        }
                    },
                    "required": ["tool", "input"]
                }),
            ),

            tool(
                "agent007_etr_list",
                "List all available ETR tools with their input/output schemas. Call this before \
                 agent007_etr_call to discover what tools are available and their required parameters.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "layer": {
                            "type": "string",
                            "description": "Filter by layer: 'l1', 'l2', 'l3', or 'all' (default)",
                            "default": "all"
                        }
                    }
                }),
            ),

            // Workflow create — save a new workflow YAML to disk
            tool(
                "agent007_workflow_create",
                "Save a new workflow YAML to ~/.agent007/workflows/<name>.yaml (or the project-local \
                 .agent007/workflows/ if one exists). The YAML must follow the agent007 workflow schema: \
                 top-level `name`, `description`, and `steps` array. Each step needs `id`, `agent`, \
                 `prompt`, `output`, and optional `depends_on`. Use agent007_workflow_list to verify \
                 the workflow appears after saving, then agent007_workflow_start or agent007_workflow_run \
                 to execute it.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Workflow name — used as the filename (kebab-case recommended, e.g. 'my-workflow')"
                        },
                        "yaml": {
                            "type": "string",
                            "description": "Complete workflow YAML content"
                        },
                        "overwrite": {
                            "type": "boolean",
                            "description": "If true, overwrite an existing workflow with the same name. Defaults to false.",
                            "default": false
                        }
                    },
                    "required": ["name", "yaml"]
                }),
            ),
        ]
    }

    fn tool_defs() -> Vec<Tool> {
        let mut defs = Self::base_tool_defs();
        defs.extend(dynamic_skill_tool_defs());
        defs.extend(dynamic_workflow_tool_defs());
        defs
    }
}

impl ServerHandler for Agent007Server {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info = Implementation::new("agent007", env!("CARGO_PKG_VERSION"))
            .with_description(
                "Multi-agent AI orchestration with RAG memory, skills, hooks, and learning.",
            );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Use agent007_help for the current catalog and exact invocation examples. \
             Use agent007_run to run tasks. Generic skill/workflow tools are available via \
             agent007_skill_list, agent007_skill_run, agent007_workflow_list, and \
             agent007_workflow_run. Discovered skills and workflows are also exposed as \
             individual MCP tools."
                .to_string(),
        );
        info
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<rmcp::model::InitializeResult, rmcp::model::ErrorData> {
        Ok(self.get_info().into())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, rmcp::model::ErrorData> {
        Ok(ListToolsResult::with_all_items(Self::tool_defs()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, rmcp::model::ErrorData> {
        if let Some(skill) = dynamic_skill_tool(request.name.as_ref()) {
            let args = string_or_default(request.arguments.as_ref(), "args", "");
            let aid = agent007_core::types::AgentId::new();
            let core_task = agent007_core::Task::new(&format!("skill:{}", skill.trigger));
            let task_id = core_task.id;
            self.publish_task_assigned(&aid, &core_task).await;
            match run_skill_mcp(&self.config, skill.trigger.clone(), args).await {
                Ok((output, tokens)) => {
                    self.publish_model_request(tokens).await;
                    self.publish_task_completed(&aid, task_id, &output).await;
                    return Ok(CallToolResult::success(vec![Content::text(output)]));
                }
                Err(e) => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))]));
                }
            }
        }

        if let Some(workflow) = dynamic_workflow_tool(request.name.as_ref()) {
            let task = extract_string(request.arguments.as_ref(), "task")?;
            let aid = agent007_core::types::AgentId::new();
            let core_task =
                agent007_core::Task::new(&format!("workflow:{}", workflow.workflow_ref));
            let task_id = core_task.id;
            self.publish_task_assigned(&aid, &core_task).await;

            let result = if standalone_mode_available(&self.config) {
                workflow_run(&self.config, &workflow.workflow_ref, &task).await
            } else {
                workflow_hosted_start(&workflow.workflow_ref, &task)
            };

            match result {
                Ok(output) => {
                    let token_est = output.len() / 4;
                    self.publish_model_request(token_est).await;
                    self.publish_task_completed(&aid, task_id, &output).await;
                    return Ok(CallToolResult::success(vec![Content::text(output)]));
                }
                Err(e) => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))]));
                }
            }
        }

        match request.name.as_ref() {
            // ── existing 5 ────────────────────────────────────────────────
            "agent007_help" => {
                let topic = optional_string(request.arguments.as_ref(), "topic");
                Ok(CallToolResult::success(vec![Content::text(agent007_help(
                    topic.as_deref(),
                ))]))
            }
            "agent007_run" => {
                let task_str = extract_string(request.arguments.as_ref(), "task")?;
                let aid = agent007_core::types::AgentId::new();
                let core_task = agent007_core::Task::new(&task_str);
                let task_id = core_task.id;
                self.publish_task_assigned(&aid, &core_task).await;
                match run_task(&self.config, task_str).await {
                    Ok(output) => {
                        let token_est = output.len() / 4;
                        self.publish_model_request(token_est).await;
                        self.publish_task_completed(&aid, task_id, &output).await;
                        Ok(CallToolResult::success(vec![Content::text(output)]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }
            "agent007_dispatch" => {
                let command = extract_string(request.arguments.as_ref(), "command")?;
                let parsed = match parse_dispatch_command(&command) {
                    Ok(parsed) => parsed,
                    Err(e) => {
                        return Ok(CallToolResult::error(vec![Content::text(format!(
                            "Error: {e}\n\n{}",
                            dispatch_usage()
                        ))]));
                    }
                };

                match parsed {
                    DispatchCommand::Help { topic } => {
                        Ok(CallToolResult::success(vec![Content::text(agent007_help(
                            topic.as_deref(),
                        ))]))
                    }
                    DispatchCommand::Run { task } => {
                        let aid = agent007_core::types::AgentId::new();
                        let core_task = agent007_core::Task::new(&task);
                        let task_id = core_task.id;
                        self.publish_task_assigned(&aid, &core_task).await;
                        match run_task(&self.config, task).await {
                            Ok(output) => {
                                let token_est = output.len() / 4;
                                self.publish_model_request(token_est).await;
                                self.publish_task_completed(&aid, task_id, &output).await;
                                Ok(CallToolResult::success(vec![Content::text(output)]))
                            }
                            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                                "Error: {e}"
                            ))])),
                        }
                    }
                    DispatchCommand::SkillList => match list_available_skills() {
                        Ok(skills) => Ok(CallToolResult::success(vec![Content::text(
                            format_skills(&skills),
                        )])),
                        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                            "Error: {e}"
                        ))])),
                    },
                    DispatchCommand::SkillRun { trigger, args } => {
                        let aid = agent007_core::types::AgentId::new();
                        let core_task = agent007_core::Task::new(&format!("skill:{trigger}"));
                        let task_id = core_task.id;
                        self.publish_task_assigned(&aid, &core_task).await;
                        self.fire_hook(&HookEvent::OnSkillExecute {
                            skill: trigger.clone(),
                        });
                        match run_skill_mcp(&self.config, trigger, args).await {
                            Ok((output, tokens)) => {
                                self.publish_model_request(tokens).await;
                                self.publish_task_completed(&aid, task_id, &output).await;
                                Ok(CallToolResult::success(vec![Content::text(output)]))
                            }
                            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                                "Error: {e}"
                            ))])),
                        }
                    }
                    DispatchCommand::WorkflowList => match workflow_list() {
                        Ok(names) => {
                            let text = if names.is_empty() {
                                format!(
                                    "No workflow files found in {}.",
                                    agent007_home().join("workflows").display()
                                )
                            } else {
                                names.join("\n")
                            };
                            Ok(CallToolResult::success(vec![Content::text(text)]))
                        }
                        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                            "Error: {e}"
                        ))])),
                    },
                    DispatchCommand::WorkflowRun { name, task } => {
                        let aid = agent007_core::types::AgentId::new();
                        let core_task = agent007_core::Task::new(&format!("workflow:{name}"));
                        let task_id = core_task.id;
                        self.publish_task_assigned(&aid, &core_task).await;
                        match workflow_run(&self.config, &name, &task).await {
                            Ok(output) => {
                                let token_est = output.len() / 4;
                                self.publish_model_request(token_est).await;
                                self.publish_task_completed(&aid, task_id, &output).await;
                                Ok(CallToolResult::success(vec![Content::text(output)]))
                            }
                            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                                "Error: {e}"
                            ))])),
                        }
                    }
                }
            }
            "agent007_skill_list" => match list_available_skills() {
                Ok(skills) => Ok(CallToolResult::success(vec![Content::text(format_skills(
                    &skills,
                ))])),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {e}"
                ))])),
            },
            "agent007_skill_run" => {
                let trigger = extract_string(request.arguments.as_ref(), "trigger")?;
                let args = string_or_default(request.arguments.as_ref(), "args", "");
                let aid = agent007_core::types::AgentId::new();
                let core_task = agent007_core::Task::new(&format!("skill:{trigger}"));
                let task_id = core_task.id;
                self.publish_task_assigned(&aid, &core_task).await;
                self.fire_hook(&HookEvent::OnSkillExecute {
                    skill: trigger.clone(),
                });
                match run_skill_mcp(&self.config, trigger, args).await {
                    Ok((output, tokens)) => {
                        self.publish_model_request(tokens).await;
                        self.publish_task_completed(&aid, task_id, &output).await;
                        Ok(CallToolResult::success(vec![Content::text(output)]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }
            "agent007_persona_list" => {
                let registry = configured_persona_registry();
                use agent007_core::PersonaProvider;
                let personas = registry.list();
                let text = if personas.is_empty() {
                    "No personas available.".to_string()
                } else {
                    personas
                        .iter()
                        .map(|p| {
                            format!("• {} [{}] — {}", p.name, p.preferred_model, p.description)
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            "agent007_persona_show" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let registry = configured_persona_registry();
                use agent007_core::PersonaProvider;
                match registry.get(&name) {
                    Some(spec) => {
                        let tools = if spec.allowed_tools.is_empty() {
                            "none".to_string()
                        } else {
                            spec.allowed_tools.join(", ")
                        };
                        let text = format!(
                            "Name: {}\nModel: {}\nDescription: {}\nAgent type: {}\nMemory namespace: {}\nAllowed workers: {}\nSkills: {}\nAllowed tools: {}\n\nSystem prompt:\n{}",
                            spec.name,
                            spec.preferred_model,
                            spec.description,
                            spec.agent_type.as_deref().unwrap_or("worker"),
                            spec.memory_namespace.as_deref().unwrap_or(&spec.name),
                            spec.allowed_workers.clone().unwrap_or_default().join(", "),
                            spec.skills.join(", "),
                            tools,
                            spec.system_prompt
                        );
                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                    None => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Persona '{}' not found.",
                        name
                    ))])),
                }
            }

            // ── new tools ─────────────────────────────────────────────────

            // 1. Memory read
            "agent007_memory_read" => {
                let scope = extract_string(request.arguments.as_ref(), "scope")?;
                let key = extract_string(request.arguments.as_ref(), "key")?;
                match memory_read(&scope, &key) {
                    Ok(Some(val)) => Ok(CallToolResult::success(vec![Content::text(val)])),
                    Ok(None) => Ok(CallToolResult::success(vec![Content::text(format!(
                        "Key '{}' not found in scope '{}'.",
                        key, scope
                    ))])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 2. Memory write
            "agent007_memory_write" => {
                let scope = extract_string(request.arguments.as_ref(), "scope")?;
                let key = extract_string(request.arguments.as_ref(), "key")?;
                let value = extract_string(request.arguments.as_ref(), "value")?;
                match memory_write(&scope, &key, &value) {
                    Ok(()) => {
                        self.fire_hook(&HookEvent::OnMemoryWrite { key: key.clone() });
                        Ok(CallToolResult::success(vec![Content::text(format!(
                            "Written key '{}' in scope '{}'.",
                            key, scope
                        ))]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 3. Memory list
            "agent007_memory_list" => {
                let scope = extract_string(request.arguments.as_ref(), "scope")?;
                match memory_list(&scope) {
                    Ok(keys) => {
                        let text = if keys.is_empty() {
                            format!("No keys found in scope '{}'.", scope)
                        } else {
                            keys.join("\n")
                        };
                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 4. Record actual tokens
            "agent007_record_tokens" => {
                let run_id = extract_string(request.arguments.as_ref(), "run_id")?;
                let tokens = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let model = extract_string(request.arguments.as_ref(), "model")?;
                let output = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("output"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                match record_actual_tokens(&run_id, tokens, &model, output.as_deref()) {
                    Ok(message) => {
                        let skill_hint =
                            load_run_store().load_run(&run_id).ok().and_then(|detail| {
                                if detail.metadata.kind == "skill" {
                                    detail
                                        .metadata
                                        .task
                                        .split_whitespace()
                                        .next()
                                        .map(|value| value.to_string())
                                } else {
                                    None
                                }
                            });
                        // Passively record feedback in the learning store
                        record_feedback_entry(&model, skill_hint.as_deref());
                        Ok(CallToolResult::success(vec![Content::text(message)]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 5. Workflow list
            "agent007_workflow_list" => match workflow_list() {
                Ok(names) => {
                    let text = if names.is_empty() {
                        format!(
                            "No workflow files found in {}.",
                            agent007_home().join("workflows").display()
                        )
                    } else {
                        names.join("\n")
                    };
                    Ok(CallToolResult::success(vec![Content::text(text)]))
                }
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {e}"
                ))])),
            },

            // 5. Workflow run
            "agent007_workflow_run" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let task = extract_string(request.arguments.as_ref(), "task")?;
                let aid = agent007_core::types::AgentId::new();
                let core_task = agent007_core::Task::new(&format!("workflow:{name}"));
                let task_id = core_task.id;
                self.publish_task_assigned(&aid, &core_task).await;
                match workflow_run(&self.config, &name, &task).await {
                    Ok(output) => {
                        let token_est = output.len() / 4;
                        self.publish_model_request(token_est).await;
                        self.publish_task_completed(&aid, task_id, &output).await;
                        Ok(CallToolResult::success(vec![Content::text(output)]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_workflow_resume" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                let aid = agent007_core::types::AgentId::new();
                let core_task = agent007_core::Task::new(&format!("workflow-resume:{session}"));
                let task_id = core_task.id;
                self.publish_task_assigned(&aid, &core_task).await;
                match workflow_resume(&self.config, &session).await {
                    Ok(output) => {
                        let token_est = output.len() / 4;
                        self.publish_model_request(token_est).await;
                        self.publish_task_completed(&aid, task_id, &output).await;
                        Ok(CallToolResult::success(vec![Content::text(output)]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_workflow_approve" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                let step = optional_string(request.arguments.as_ref(), "step");
                let decision = extract_string(request.arguments.as_ref(), "decision")?;
                let content = optional_string(request.arguments.as_ref(), "content");
                match workflow_approve(&session, step, &decision, content) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_workflow_start" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let task = extract_string(request.arguments.as_ref(), "task")?;
                match workflow_hosted_start(&name, &task) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_workflow_next" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                match workflow_hosted_next(&session) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_workflow_submit_step" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                let step = extract_string(request.arguments.as_ref(), "step")?;
                let output = extract_string(request.arguments.as_ref(), "output")?;
                let tokens = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("tokens"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                match workflow_hosted_submit_step(&session, &step, &output, tokens) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_workflow_status" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                match workflow_hosted_status(&session) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_workflow_get_output" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                let key = extract_string(request.arguments.as_ref(), "key")?;
                match workflow_hosted_get_output(&session, &key) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_workflow_heartbeat" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                let step = extract_string(request.arguments.as_ref(), "step")?;
                let hint = optional_string(request.arguments.as_ref(), "hint");
                match workflow_hosted_heartbeat(&session, &step, hint.as_deref()) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 6. Git status
            "agent007_git_status" => match git_run(&["status"]) {
                Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {e}"
                ))])),
            },

            // 7. Git diff
            "agent007_git_diff" => {
                // Show both unstaged and staged diffs
                let unstaged = git_run(&["diff"]).unwrap_or_default();
                let staged = git_run(&["diff", "--staged"]).unwrap_or_default();
                let combined = format!(
                    "=== Unstaged ===\n{}\n=== Staged ===\n{}",
                    if unstaged.is_empty() {
                        "(none)"
                    } else {
                        &unstaged
                    },
                    if staged.is_empty() { "(none)" } else { &staged },
                );
                Ok(CallToolResult::success(vec![Content::text(combined)]))
            }

            // 8. Git log
            "agent007_git_log" => {
                let n = number_or_default(request.arguments.as_ref(), "n", 10);
                let n_str = n.to_string();
                match git_run(&["log", "--oneline", &format!("-{}", n_str)]) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 9. Git commit
            "agent007_git_commit" => {
                let message = extract_string(request.arguments.as_ref(), "message")?;
                match git_run(&["commit", "-m", &message]) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 10. Persona switch
            "agent007_persona_switch" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                // Validate the persona exists
                let registry = configured_persona_registry();
                use agent007_core::PersonaProvider;
                if registry.get(&name).is_none() {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Persona '{}' not found.",
                        name
                    ))]));
                }
                // Store in memory under scope "user", key "active_persona"
                match memory_write("user", "active_persona", &name) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!(
                        "Active persona switched to '{}'.",
                        name
                    ))])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 11. Zone check
            "agent007_zone_check" => {
                let path_str = extract_string(request.arguments.as_ref(), "path")?;
                let operation = extract_string(request.arguments.as_ref(), "operation")?;
                match zone_check(&self.config, &path_str, &operation) {
                    Ok(result) => Ok(CallToolResult::success(vec![Content::text(result)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 12. Task submit
            "agent007_task_submit" => {
                let task_str = extract_string(request.arguments.as_ref(), "task")?;
                let persona = string_or_default(request.arguments.as_ref(), "persona", "");
                let aid = agent007_core::types::AgentId::new();
                let core_task = agent007_core::Task::new(&task_str);
                let task_id = core_task.id;
                self.publish_task_assigned(&aid, &core_task).await;
                match task_submit(
                    &self.config,
                    task_str,
                    if persona.is_empty() {
                        None
                    } else {
                        Some(persona)
                    },
                )
                .await
                {
                    Ok(output) => {
                        self.publish_task_completed(&aid, task_id, &output).await;
                        Ok(CallToolResult::success(vec![Content::text(output)]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 13. Skill create
            "agent007_skill_create" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let trigger = extract_string(request.arguments.as_ref(), "trigger")?;
                let description = extract_string(request.arguments.as_ref(), "description")?;
                let template = extract_string(request.arguments.as_ref(), "template")?;
                let default_model = self.config.models.default_provider();
                let model =
                    string_or_default(request.arguments.as_ref(), "model", default_model.as_str());
                match skill_create(&name, &trigger, &description, &template, &model) {
                    Ok(path) => Ok(CallToolResult::success(vec![Content::text(format!(
                        "Skill '{}' created at {}.",
                        name, path
                    ))])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 14. Config show
            "agent007_config_show" => {
                let text = config_show(&self.config);
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }

            // 15. Health
            "agent007_health" => {
                let text = health_check(&self.config);
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }

            // 16. Workflow plan (delegate mode)
            "agent007_workflow_plan" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let task = extract_string(request.arguments.as_ref(), "task")?;
                match workflow_plan(&self.config, &name, &task).await {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 17. Agent create (wizard mode)
            "agent007_agent_create" => {
                let action = extract_string(request.arguments.as_ref(), "action")?;
                match action.as_str() {
                    "catalog" => {
                        let text = agent_catalog();
                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                    "save" => {
                        let name = extract_string(request.arguments.as_ref(), "name")?;
                        let description =
                            extract_string(request.arguments.as_ref(), "description")?;
                        let system_prompt =
                            extract_string(request.arguments.as_ref(), "system_prompt")?;
                        let default_provider = self.config.models.default_provider();
                        let preferred_model = string_or_default(
                            request.arguments.as_ref(),
                            "preferred_model",
                            default_provider.as_str(),
                        );
                        let allowed_tools =
                            optional_string_array_arg(request.arguments.as_ref(), "allowed_tools");
                        let skills =
                            optional_string_array_arg(request.arguments.as_ref(), "skills");
                        let allowed_workers = optional_string_array_arg(
                            request.arguments.as_ref(),
                            "allowed_workers",
                        );
                        let agent_type = request
                            .arguments
                            .as_ref()
                            .and_then(|a| a.get("agent_type"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                        let memory_namespace = request
                            .arguments
                            .as_ref()
                            .and_then(|a| a.get("memory_namespace"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                        match agent_save(
                            &name,
                            &description,
                            &system_prompt,
                            &preferred_model,
                            allowed_tools.as_deref(),
                            skills.as_deref(),
                            agent_type.as_deref(),
                            allowed_workers.as_deref(),
                            memory_namespace.as_deref(),
                        ) {
                            Ok(path) => Ok(CallToolResult::success(vec![Content::text(
                                format!("Agent '{}' saved to {}. It is now available for workflows and orchestration.", name, path)
                            )])),
                            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                        }
                    }
                    other => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Unknown action '{}'. Use 'catalog' or 'save'.",
                        other
                    ))])),
                }
            }

            // 18. Skill wizard
            "agent007_skill_wizard" => {
                let action = extract_string(request.arguments.as_ref(), "action")?;
                match action.as_str() {
                    "templates" => {
                        let text = skill_templates();
                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                    "save" => {
                        let name = extract_string(request.arguments.as_ref(), "name")?;
                        let trigger = extract_string(request.arguments.as_ref(), "trigger")?;
                        let description =
                            extract_string(request.arguments.as_ref(), "description")?;
                        let template = extract_string(request.arguments.as_ref(), "template")?;
                        let default_model = self.config.models.default_provider();
                        let model = string_or_default(
                            request.arguments.as_ref(),
                            "model",
                            default_model.as_str(),
                        );
                        match skill_create(&name, &trigger, &description, &template, &model) {
                            Ok(path) => Ok(CallToolResult::success(vec![Content::text(format!(
                                "Skill '{}' created at {}. Trigger: {}",
                                name, path, trigger
                            ))])),
                            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                                "Error: {e}"
                            ))])),
                        }
                    }
                    other => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Unknown action '{}'. Use 'templates' or 'save'.",
                        other
                    ))])),
                }
            }

            // Workflow create
            "agent007_workflow_create" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let yaml = extract_string(request.arguments.as_ref(), "yaml")?;
                let overwrite = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("overwrite"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                match workflow_create(&name, &yaml, overwrite) {
                    Ok(path) => Ok(CallToolResult::success(vec![Content::text(
                        format!("Workflow '{}' saved to {}. Use agent007_workflow_list to confirm, then agent007_workflow_start or agent007_workflow_run to execute it.", name, path)
                    )])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 19. Downstream MCP tool list
            "agent007_mcp_tools_list" => match mcp_tools_list(&self.config).await {
                Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {e}"
                ))])),
            },

            // 20. Downstream MCP tool call
            "agent007_mcp_tool_call" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let args = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("args"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let aid = agent007_core::types::AgentId::new();
                match mcp_tool_call(&self.config, &aid, &name, args).await {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 21. Run history
            "agent007_run_history" => {
                let limit = number_or_default(request.arguments.as_ref(), "limit", 10) as usize;
                match run_history(limit) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // 22. Run show
            "agent007_run_show" => {
                let id = extract_string(request.arguments.as_ref(), "id")?;
                match run_show(&id) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_compact_output" => {
                let command = extract_string(request.arguments.as_ref(), "command")?;
                let output = extract_string(request.arguments.as_ref(), "output")?;
                let level = string_or_default(request.arguments.as_ref(), "level", "compact");
                match compact_output_tool(&self.config, &command, &output, &level) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_context_compile" => {
                let task = extract_string(request.arguments.as_ref(), "task")?;
                let max_files =
                    number_or_default(request.arguments.as_ref(), "max_files", 8) as usize;
                let max_memory_notes =
                    number_or_default(request.arguments.as_ref(), "max_memory_notes", 6) as usize;
                let max_prompt_tokens =
                    number_or_default(request.arguments.as_ref(), "max_prompt_tokens", 8_000);
                let reserve_tokens =
                    number_or_default(request.arguments.as_ref(), "reserve_tokens", 1_500);
                let max_response_tokens =
                    number_or_default(request.arguments.as_ref(), "max_response_tokens", 2_000);
                match context_compile_tool(
                    &self.config,
                    &task,
                    max_files,
                    max_memory_notes,
                    max_prompt_tokens,
                    reserve_tokens,
                    max_response_tokens,
                ) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            "agent007_repo_brain_refresh" => match repo_brain_refresh_tool(&self.config) {
                Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {e}"
                ))])),
            },

            "agent007_budget_estimate" => {
                let task = optional_string(request.arguments.as_ref(), "task");
                let text = optional_string(request.arguments.as_ref(), "text");
                let max_prompt_tokens =
                    number_or_default(request.arguments.as_ref(), "max_prompt_tokens", 8_000);
                let reserve_tokens =
                    number_or_default(request.arguments.as_ref(), "reserve_tokens", 1_500);
                let max_response_tokens =
                    number_or_default(request.arguments.as_ref(), "max_response_tokens", 2_000);
                match budget_estimate_tool(
                    &self.config,
                    task,
                    text,
                    max_prompt_tokens,
                    reserve_tokens,
                    max_response_tokens,
                ) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {e}"
                    ))])),
                }
            }

            // ETR — Embedded Tool Runtime
            "agent007_etr_call" => {
                let tool_name = extract_string(request.arguments.as_ref(), "tool")?;
                let input = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("input"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let compact = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("compact"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let result = etr_call(&tool_name, input, compact);
                Ok(CallToolResult::success(vec![Content::text(result)]))
            }

            "agent007_etr_list" => {
                let result = etr_list();
                Ok(CallToolResult::success(vec![Content::text(result)]))
            }

            // Multi-agent execution
            "agent007_agent_run" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let task = extract_string(request.arguments.as_ref(), "task")?;

                let stack = match build_stack(&self.config).await {
                    Ok(s) => s,
                    Err(e) => {
                        return Ok(CallToolResult::error(vec![Content::text(format!(
                            "Error initialising agent stack: {e}"
                        ))]));
                    }
                };

                // Prefer the server's shared dispatcher so worker events
                // (WorkerResult/WorkerBlocked) reach the same dashboard stream
                // as TaskAssigned/TaskCompleted.
                use agent007_core::dispatcher::Dispatcher;
                let run_dispatcher: Arc<dyn Dispatcher> = self
                    .dispatcher
                    .clone()
                    .map(|d| d as Arc<dyn Dispatcher>)
                    .unwrap_or_else(|| stack.dispatcher as Arc<dyn Dispatcher>);

                let persona_provider =
                    stack.persona_registry as Arc<dyn agent007_core::persona::PersonaProvider>;
                let orch = if let Some(persona) = persona_provider.get(&name) {
                    if !matches!(
                        persona.agent_type.as_deref(),
                        Some(kind) if kind.eq_ignore_ascii_case("orchestrator")
                    ) {
                        return Ok(CallToolResult::error(vec![Content::text(format!(
                            "Persona '{}' must have agent_type = 'orchestrator' to run as an agent.",
                            name
                        ))]));
                    }
                    let ns = persona
                        .memory_namespace
                        .clone()
                        .unwrap_or_else(|| persona.name.clone());
                    let scoped = Arc::new(stack.memory_store.scoped(&ns));
                    let skills_dir = agent007_home().join("skills");
                    let skill_provider: Arc<dyn agent007_skills::SkillContentProvider> =
                        match agent007_skills::SkillLoader::new(&skills_dir).load_all() {
                            Ok(skills) => {
                                Arc::new(agent007_skills::SkillIndex::from_skills(skills))
                            }
                            Err(_) => Arc::new(agent007_skills::NoOpSkillContentProvider),
                        };
                    agent007_custom_agents::SubOrchestrator::from_persona(
                        &persona,
                        Vec::new(),
                        skill_provider,
                        scoped,
                        stack.model_router,
                        persona_provider,
                        run_dispatcher,
                        0,
                        3,
                    )
                } else {
                    let agents_dir = agent007_home().join("agents");
                    let registry = Arc::new(
                        agent007_custom_agents::AgentRegistry::load(&agents_dir)
                            .unwrap_or_else(|_| agent007_custom_agents::AgentRegistry::empty()),
                    );
                    let def = match registry.get(&name) {
                        Some(d) => d.clone(),
                        None => {
                            return Ok(CallToolResult::error(vec![Content::text(format!(
                                "Agent or orchestrator persona '{}' not found.",
                                name
                            ))]));
                        }
                    };
                    let ns = def
                        .memory_namespace
                        .clone()
                        .unwrap_or_else(|| def.name.clone());
                    let scoped = Arc::new(stack.memory_store.scoped(&ns));
                    agent007_custom_agents::SubOrchestrator::new(
                        def,
                        scoped,
                        stack.model_router,
                        persona_provider,
                        run_dispatcher,
                        0,
                        3,
                    )
                };

                let aid = agent007_core::types::AgentId::new();
                let core_task = agent007_core::Task::new(&format!("agent:{name}:{task}"));
                let task_id = core_task.id;
                self.publish_task_assigned(&aid, &core_task).await;

                match orch.run(&task).await {
                    Ok(result) => {
                        let mut lines = vec![result.output.clone()];
                        if !result.blockers.is_empty() {
                            lines.push("\nBlockers:".to_string());
                            for b in &result.blockers {
                                lines.push(format!("  • {b}"));
                            }
                        }
                        if !result.files_changed.is_empty() {
                            lines.push("\nFiles changed:".to_string());
                            for f in &result.files_changed {
                                lines.push(format!("  • {}", f.display()));
                            }
                        }
                        let output = lines.join("\n");
                        let token_est = output.len() / 4;
                        self.publish_model_request(token_est).await;
                        self.publish_task_completed(&aid, task_id, &output).await;
                        Ok(CallToolResult::success(vec![Content::text(output)]))
                    }
                    Err(e) => {
                        // Publish a terminal event so the dashboard doesn't show
                        // this run stuck in "running" state indefinitely.
                        if let Some(d) = &self.dispatcher {
                            let _ = d
                                .publish(agent007_core::AgentEvent::TaskFailed {
                                    agent_id: aid.clone(),
                                    error: e.to_string(),
                                    model: None,
                                })
                                .await;
                        }
                        Ok(CallToolResult::error(vec![Content::text(format!(
                            "Agent run failed: {e}"
                        ))]))
                    }
                }
            }

            name => Err(rmcp::model::ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("Unknown tool: {name}"),
                None,
            )),
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Build a `Tool` from a JSON schema Value (must be an object).
fn tool(name: &'static str, description: &'static str, schema: serde_json::Value) -> Tool {
    let map: Map<String, serde_json::Value> = match schema {
        serde_json::Value::Object(m) => m,
        _ => Map::new(),
    };
    Tool::new(name, description, Arc::new(map))
}

fn tool_owned(
    name: impl Into<String>,
    description: impl Into<String>,
    schema: serde_json::Value,
) -> Tool {
    let map: Map<String, serde_json::Value> = match schema {
        serde_json::Value::Object(m) => m,
        _ => Map::new(),
    };
    Tool::new(name.into(), description.into(), Arc::new(map))
}

fn extract_string(
    args: Option<&Map<String, serde_json::Value>>,
    key: &str,
) -> std::result::Result<String, rmcp::model::ErrorData> {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            rmcp::model::ErrorData::new(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                format!("Missing required parameter: {key}"),
                None,
            )
        })
}

fn optional_string(args: Option<&Map<String, serde_json::Value>>, key: &str) -> Option<String> {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn optional_string_array_arg(
    args: Option<&Map<String, serde_json::Value>>,
    key: &str,
) -> Option<Vec<String>> {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
}

fn string_or_default(
    args: Option<&Map<String, serde_json::Value>>,
    key: &str,
    default: &str,
) -> String {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}

fn number_or_default(
    args: Option<&Map<String, serde_json::Value>>,
    key: &str,
    default: u64,
) -> u64 {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_u64())
        .unwrap_or(default)
}

fn parse_compact_level(level: &str) -> Result<agent007_core::CompactLevel> {
    match level.trim().to_lowercase().as_str() {
        "full" => Ok(agent007_core::CompactLevel::Full),
        "compact" | "" => Ok(agent007_core::CompactLevel::Compact),
        "aggressive" => Ok(agent007_core::CompactLevel::Aggressive),
        other => Err(anyhow::anyhow!(
            "unknown compact level '{}'; use full, compact, or aggressive",
            other
        )),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DispatchCommand {
    Help { topic: Option<String> },
    Run { task: String },
    SkillList,
    SkillRun { trigger: String, args: String },
    WorkflowList,
    WorkflowRun { name: String, task: String },
}

fn split_first_token(input: &str) -> (&str, &str) {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return ("", "");
    }
    let boundary = trimmed
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(idx, _)| idx)
        .unwrap_or(trimmed.len());
    let head = &trimmed[..boundary];
    let tail = trimmed[boundary..].trim_start();
    (head, tail)
}

fn strip_agent007_prefix(input: &str) -> &str {
    let (head, tail) = split_first_token(input);
    match head.to_ascii_lowercase().as_str() {
        "$agent007" | "/agent007" | "@agent007" | "agent007" => tail,
        _ => input.trim(),
    }
}

fn canonicalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

fn resolve_workflow_alias(alias: &str) -> Option<String> {
    let canonical_alias = canonicalize_identifier(alias);
    let sanitized_alias = sanitize_tool_component(alias);
    load_available_workflows().ok().and_then(|workflows| {
        workflows.into_iter().find_map(|(workflow_ref, _)| {
            let canonical_ref = canonicalize_identifier(&workflow_ref);
            let sanitized_ref = sanitize_tool_component(&workflow_ref);
            if canonical_ref == canonical_alias || sanitized_ref == sanitized_alias {
                Some(workflow_ref)
            } else {
                None
            }
        })
    })
}

fn normalize_dispatch_skill_trigger(trigger: &str) -> String {
    let trimmed = trigger.trim();
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed.trim_start_matches('/'))
    }
}

fn parse_dispatch_command(command: &str) -> Result<DispatchCommand> {
    let normalized = strip_agent007_prefix(command).trim();
    if normalized.is_empty() {
        return Ok(DispatchCommand::Help {
            topic: Some("overview".to_string()),
        });
    }

    let (head, tail) = split_first_token(normalized);
    let head_lower = head.to_ascii_lowercase();

    match head_lower.as_str() {
        "help" | "?" | "-h" | "--help" => {
            let (topic, _) = split_first_token(tail);
            let topic = match topic.to_ascii_lowercase().as_str() {
                "overview" | "skills" | "workflows" | "tools" => Some(topic.to_ascii_lowercase()),
                _ => None,
            };
            Ok(DispatchCommand::Help { topic })
        }
        "run" => {
            if tail.is_empty() {
                return Err(anyhow::anyhow!("run requires a task"));
            }
            Ok(DispatchCommand::Run {
                task: tail.to_string(),
            })
        }
        "workflow" | "workflows" | "wf" => {
            let (name, task) = split_first_token(tail);
            if name.is_empty() || matches!(name.to_ascii_lowercase().as_str(), "list" | "ls") {
                return Ok(DispatchCommand::WorkflowList);
            }
            if task.is_empty() {
                return Err(anyhow::anyhow!("workflow '{}' requires a task", name));
            }
            Ok(DispatchCommand::WorkflowRun {
                name: resolve_workflow_alias(name).unwrap_or_else(|| name.to_string()),
                task: task.to_string(),
            })
        }
        "skill" | "skills" | "sk" => {
            let (trigger, args) = split_first_token(tail);
            if trigger.is_empty() || matches!(trigger.to_ascii_lowercase().as_str(), "list" | "ls")
            {
                return Ok(DispatchCommand::SkillList);
            }
            Ok(DispatchCommand::SkillRun {
                trigger: normalize_dispatch_skill_trigger(trigger),
                args: args.to_string(),
            })
        }
        _ => {
            if head.starts_with('/') {
                return Ok(DispatchCommand::SkillRun {
                    trigger: normalize_dispatch_skill_trigger(head),
                    args: tail.to_string(),
                });
            }
            if let Some(workflow_ref) = resolve_workflow_alias(head) {
                if tail.is_empty() {
                    return Err(anyhow::anyhow!(
                        "workflow '{}' requires a task",
                        workflow_ref
                    ));
                }
                return Ok(DispatchCommand::WorkflowRun {
                    name: workflow_ref,
                    task: tail.to_string(),
                });
            }
            Ok(DispatchCommand::Run {
                task: normalized.to_string(),
            })
        }
    }
}

fn dispatch_usage() -> &'static str {
    "Usage examples:\n\
     - $agent007 wf tdd add login rate limiting\n\
     - $agent007 workflow code-review review the current diff\n\
     - $agent007 skill /brainstorm onboarding ideas\n\
     - $agent007 /dev-pr-review review this patch\n\
     - $agent007 run refactor auth module\n\
     - /agent007 help workflows"
}

#[derive(Clone, Debug)]
struct DynamicSkillTool {
    tool_name: String,
    legacy_tool_names: Vec<String>,
    trigger: String,
    name: String,
    description: String,
}

#[derive(Clone, Debug)]
struct DynamicWorkflowTool {
    tool_name: String,
    workflow_ref: String,
    display_name: String,
    description: Option<String>,
}

fn configured_agent007_homes() -> Vec<PathBuf> {
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        return vec![PathBuf::from(home)];
    }

    let mut homes = Vec::new();
    if let Some(project_home) = agent007_project_home() {
        homes.push(project_home);
    }
    let global_home = agent007_global_home();
    if !homes.iter().any(|home| home == &global_home) {
        homes.push(global_home);
    }
    homes
}

fn configured_skill_dirs() -> Vec<PathBuf> {
    configured_agent007_homes()
        .into_iter()
        .map(|home| home.join("skills"))
        .collect()
}

fn configured_workflow_dirs() -> Vec<PathBuf> {
    configured_agent007_homes()
        .into_iter()
        .map(|home| home.join("workflows"))
        .collect()
}

fn configured_persona_dirs() -> Vec<PathBuf> {
    configured_agent007_homes()
        .into_iter()
        .map(|home| home.join("personas"))
        .collect()
}

fn configured_persona_registry() -> agent007_personas::PersonaRegistry {
    let dirs = configured_persona_dirs();
    agent007_personas::PersonaRegistry::load_from_dirs(dirs.iter().map(|dir| dir.as_path()))
        .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in())
}

fn sanitize_tool_component(value: &str) -> String {
    let mut out = String::new();
    let mut previous_was_separator = false;

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            out.push('_');
            previous_was_separator = true;
        }
    }

    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "item".to_string()
    } else {
        trimmed
    }
}

fn normalize_skill_trigger_for_tool(trigger: &str) -> String {
    let trimmed = trigger.trim().trim_start_matches('/');
    let normalized = trimmed
        .strip_prefix("agent007/")
        .or_else(|| trimmed.strip_prefix("agent007-"))
        .or_else(|| trimmed.strip_prefix("agent007_"))
        .unwrap_or(trimmed);
    sanitize_tool_component(normalized)
}

fn load_available_skills() -> Result<Vec<agent007_skills::Skill>> {
    let mut skills = BTreeMap::new();

    for skills_dir in configured_skill_dirs() {
        if !skills_dir.exists() {
            continue;
        }

        let loader = agent007_skills::SkillLoader::new(&skills_dir);
        for skill in loader.load_all().map_err(|e| {
            anyhow::anyhow!("failed to load skills from {}: {}", skills_dir.display(), e)
        })? {
            skills.entry(skill.trigger().to_string()).or_insert(skill);
        }
    }

    Ok(skills.into_values().collect())
}

fn catalog_collision_counts() -> (usize, usize) {
    let mut skill_counts: HashMap<String, usize> = HashMap::new();
    for skills_dir in configured_skill_dirs() {
        if !skills_dir.exists() {
            continue;
        }
        let loader = agent007_skills::SkillLoader::new(&skills_dir);
        if let Ok(skills) = loader.load_all() {
            for skill in skills {
                *skill_counts.entry(skill.trigger().to_string()).or_insert(0) += 1;
            }
        }
    }
    let skill_collisions = skill_counts.values().filter(|count| **count > 1).count();

    let mut workflow_counts: HashMap<String, usize> = HashMap::new();
    for workflows_dir in configured_workflow_dirs() {
        if !workflows_dir.exists() {
            continue;
        }
        let loader = agent007_workflows::WorkflowLoader::new(workflows_dir.clone());
        if let Ok(names) = loader.list_names() {
            for workflow in names {
                *workflow_counts.entry(workflow).or_insert(0) += 1;
            }
        }
    }
    let workflow_collisions = workflow_counts.values().filter(|count| **count > 1).count();

    (skill_collisions, workflow_collisions)
}

fn list_available_skills() -> Result<Vec<SkillSummary>> {
    Ok(load_available_skills()?
        .into_iter()
        .map(|skill| SkillSummary {
            name: skill.name().to_string(),
            description: skill.frontmatter.description.clone(),
            trigger: skill.trigger().to_string(),
            version: skill.version().to_string(),
        })
        .collect())
}

fn find_skill(trigger: &str) -> Result<agent007_skills::Skill> {
    load_available_skills()?
        .into_iter()
        .find(|skill| skill.trigger() == trigger)
        .ok_or_else(|| anyhow::anyhow!("no skill found with trigger: {}", trigger))
}

fn dynamic_skill_catalog() -> Vec<DynamicSkillTool> {
    let mut tools = BTreeMap::new();

    if let Ok(skills) = load_available_skills() {
        for skill in skills {
            let normalized = normalize_skill_trigger_for_tool(skill.trigger());
            let legacy = sanitize_tool_component(skill.trigger().trim_start_matches('/'));
            let tool_name = format!("agent007_skill_{normalized}");
            let legacy_tool_names = if legacy != normalized {
                vec![format!("agent007_skill_{legacy}")]
            } else {
                Vec::new()
            };
            tools.entry(tool_name.clone()).or_insert(DynamicSkillTool {
                tool_name,
                legacy_tool_names,
                trigger: skill.trigger().to_string(),
                name: skill.name().to_string(),
                description: skill.frontmatter.description.clone(),
            });
        }
    }

    tools.into_values().collect()
}

fn dynamic_skill_tool_defs() -> Vec<Tool> {
    dynamic_skill_catalog()
        .into_iter()
        .map(|skill| {
            tool_owned(
                skill.tool_name,
                format!(
                    "Run skill '{}' (trigger {}). {}",
                    skill.name, skill.trigger, skill.description
                ),
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "args": {
                            "type": "string",
                            "description": "Arguments to inject into the skill template",
                            "default": ""
                        }
                    }
                }),
            )
        })
        .collect()
}

fn dynamic_skill_tool(name: &str) -> Option<DynamicSkillTool> {
    dynamic_skill_catalog().into_iter().find(|tool| {
        tool.tool_name == name || tool.legacy_tool_names.iter().any(|alias| alias == name)
    })
}

fn load_available_workflows() -> Result<Vec<(String, agent007_workflows::WorkflowDef)>> {
    let mut workflows = BTreeMap::new();

    for workflows_dir in configured_workflow_dirs() {
        if !workflows_dir.exists() {
            continue;
        }

        let loader = agent007_workflows::WorkflowLoader::new(workflows_dir.clone());
        for workflow_ref in loader.list_names().map_err(|e| {
            anyhow::anyhow!(
                "failed to list workflows from {}: {}",
                workflows_dir.display(),
                e
            )
        })? {
            if workflows.contains_key(&workflow_ref) {
                continue;
            }
            let def = loader.load_named(&workflow_ref).map_err(|e| {
                anyhow::anyhow!("failed to load workflow '{}': {}", workflow_ref, e)
            })?;
            workflows.insert(workflow_ref, def);
        }
    }

    Ok(workflows.into_iter().collect())
}

fn dynamic_workflow_catalog() -> Vec<DynamicWorkflowTool> {
    let mut tools = BTreeMap::new();

    if let Ok(workflows) = load_available_workflows() {
        for (workflow_ref, def) in workflows {
            let tool_name = format!(
                "agent007_workflow_{}",
                sanitize_tool_component(&workflow_ref)
            );
            tools
                .entry(tool_name.clone())
                .or_insert(DynamicWorkflowTool {
                    tool_name,
                    workflow_ref,
                    display_name: def.name,
                    description: def.description,
                });
        }
    }

    tools.into_values().collect()
}

fn dynamic_workflow_tool_defs() -> Vec<Tool> {
    dynamic_workflow_catalog()
        .into_iter()
        .map(|workflow| {
            let mut description = format!(
                "Start workflow '{}' (ref {}). Provide a task description.",
                workflow.display_name, workflow.workflow_ref
            );
            if let Some(extra) = workflow.description.as_deref() {
                description.push(' ');
                description.push_str(extra);
            }
            tool_owned(
                workflow.tool_name,
                description,
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "Task description injected into the workflow"
                        }
                    },
                    "required": ["task"]
                }),
            )
        })
        .collect()
}

fn dynamic_workflow_tool(name: &str) -> Option<DynamicWorkflowTool> {
    dynamic_workflow_catalog()
        .into_iter()
        .find(|tool| tool.tool_name == name)
}

fn agent007_help(topic: Option<&str>) -> String {
    let skills = dynamic_skill_catalog();
    let workflows = dynamic_workflow_catalog();
    let topic = topic.unwrap_or("overview");

    let core_tools = [
        (
            "agent007_dispatch",
            "Dispatch a simple command-style request to run/skill/workflow tools",
        ),
        ("agent007_run", "Run a general task through agent007"),
        ("agent007_help", "Show this catalog and invocation guidance"),
        ("agent007_skill_list", "List installed skills"),
        ("agent007_skill_run", "Run a skill by trigger"),
        ("agent007_workflow_list", "List installed workflows"),
        ("agent007_workflow_run", "Run a workflow by name"),
        (
            "agent007_workflow_create",
            "Save a new or updated workflow YAML to disk",
        ),
        (
            "agent007_workflow_start",
            "Start a hosted MCP workflow session",
        ),
        (
            "agent007_workflow_next",
            "Get the next hosted workflow steps",
        ),
        (
            "agent007_workflow_submit_step",
            "Submit hosted workflow step output",
        ),
        ("agent007_workflow_status", "Inspect hosted workflow state"),
        (
            "agent007_workflow_get_output",
            "Fetch a named step output (use inside step agents to avoid token bloat)",
        ),
        (
            "agent007_workflow_heartbeat",
            "Report progress from inside a running step (keeps it alive in the watchdog)",
        ),
    ];

    let mut lines = Vec::new();

    if matches!(topic, "overview" | "tools") {
        lines.push("agent007 MCP Catalog".to_string());
        lines.push(String::new());
        lines.push("How to invoke from Codex: use `agent007_dispatch` for slash-like commands, or call named tools directly.".to_string());
        lines.push("Examples:".to_string());
        lines.push("- Use the MCP tool agent007_dispatch with command \"$agent007 wf tdd build login flow with tests\".".to_string());
        lines.push("- Use the MCP tool agent007_dispatch with command \"$agent007 skill /brainstorm onboarding ideas\".".to_string());
        lines.push(
            "- Use the MCP tool agent007_workflow_tdd with task \"build login flow with tests\"."
                .to_string(),
        );
        lines.push("- Call agent007_help with topic \"skills\".".to_string());
        lines.push(String::new());
        lines.push("Core tools:".to_string());
        for (name, description) in core_tools {
            lines.push(format!("- {name}: {description}"));
        }
    }

    if matches!(topic, "overview" | "skills") {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("Skills ({})", skills.len()));
        if skills.is_empty() {
            lines.push("- none discovered".to_string());
        } else {
            for skill in skills {
                let alias_note = if skill.legacy_tool_names.is_empty() {
                    String::new()
                } else {
                    format!(" [legacy aliases: {}]", skill.legacy_tool_names.join(", "))
                };
                lines.push(format!(
                    "- {}: {} (trigger {}){}",
                    skill.tool_name, skill.description, skill.trigger, alias_note
                ));
            }
        }
    }

    if matches!(topic, "overview" | "workflows") {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("Workflows ({})", workflows.len()));
        if workflows.is_empty() {
            lines.push("- none discovered".to_string());
        } else {
            for workflow in workflows {
                let description = workflow
                    .description
                    .unwrap_or_else(|| "No description".to_string());
                lines.push(format!(
                    "- {}: {} (workflow ref {})",
                    workflow.tool_name, description, workflow.workflow_ref
                ));
            }
        }
    }

    lines.join("\n")
}

fn parse_approval_decision(
    decision: &str,
    content: Option<String>,
) -> Result<agent007_workflows::approval::ApprovalDecision> {
    use agent007_workflows::approval::{ApprovalDecision, ApprovalDecisionKind};

    let kind = match decision.trim().to_lowercase().as_str() {
        "approve" | "approved" | "yes" | "y" => ApprovalDecisionKind::Approve,
        "deny" | "denied" | "no" | "n" => ApprovalDecisionKind::Deny,
        "edit" | "edited" => ApprovalDecisionKind::Edit,
        other => {
            return Err(anyhow::anyhow!(
                "unknown approval decision '{}'; use approve, deny, or edit",
                other
            ));
        }
    };

    if kind == ApprovalDecisionKind::Edit && content.as_deref().unwrap_or("").trim().is_empty() {
        return Err(anyhow::anyhow!("content is required when decision=edit"));
    }

    Ok(ApprovalDecision {
        decision: kind,
        content,
    })
}

fn format_skills(skills: &[SkillSummary]) -> String {
    if skills.is_empty() {
        return "No skills loaded. Add skills to ~/.agent007/skills/".to_string();
    }
    skills
        .iter()
        .map(|s| format!("[v{}] {} — {}", s.version, s.trigger, s.description))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── task helpers ─────────────────────────────────────────────────────────────

fn load_run_store() -> agent007_core::RunStore {
    agent007_core::RunStore::new(agent007_write_home().join("sessions"))
}

async fn create_traced_stack(
    config: &Config,
    kind: &str,
    task: &str,
) -> Result<(super::run::Stack, String)> {
    let stack = build_stack(config).await?;
    let mode = runtime_mode_label(config);
    let provider = selected_runtime_provider(config);
    let run = stack
        .run_store
        .create_run(kind, task, mode, provider.as_deref())?;
    let _trace = stack
        .run_store
        .spawn_dispatcher_trace(
            run.id.clone(),
            stack.dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        )
        .await?;
    let baseline = RetrievalTelemetryArtifact::baseline(stack.rag_warmup_indexed_docs);
    let _ = write_retrieval_telemetry(&stack.run_store, &run.id, &baseline);
    Ok((stack, run.id))
}

#[derive(Debug, Clone, serde::Serialize)]
struct RetrievalTelemetryArtifact {
    indexed_docs: usize,
    retrieval_queries: u32,
    retrieval_hits: u32,
    retrieval_hit_rate: f64,
    rag_context_chars: usize,
    vector_hits: usize,
    fallback_hits: usize,
    mock_embedding: bool,
}

impl RetrievalTelemetryArtifact {
    fn baseline(indexed_docs: usize) -> Self {
        Self {
            indexed_docs,
            retrieval_queries: 0,
            retrieval_hits: 0,
            retrieval_hit_rate: 0.0,
            rag_context_chars: 0,
            vector_hits: 0,
            fallback_hits: 0,
            mock_embedding: false,
        }
    }

    fn from_skill_report(
        indexed_docs: usize,
        report: &agent007_skills::SkillExecutionReport,
    ) -> Self {
        Self {
            indexed_docs,
            retrieval_queries: report.metrics.retrieval_queries,
            retrieval_hits: report.metrics.retrieval_hits,
            retrieval_hit_rate: report.metrics.retrieval_hit_rate,
            rag_context_chars: report.metrics.rag_context_chars,
            vector_hits: report.metrics.vector_hits,
            fallback_hits: report.metrics.fallback_hits,
            mock_embedding: report.metrics.mock_embedding,
        }
    }
}

fn write_retrieval_telemetry(
    store: &agent007_core::RunStore,
    run_id: &str,
    telemetry: &RetrievalTelemetryArtifact,
) -> Result<()> {
    store
        .write_json_artifact(run_id, "retrieval-telemetry.json", telemetry)
        .map_err(|e| anyhow::anyhow!("{}", e))
}

fn create_delegate_run(kind: &str, task: &str) -> Result<String> {
    let run = load_run_store().create_run(kind, task, "hosted-mcp", None)?;
    Ok(run.id)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct HostedDelegateState {
    awaiting_host_report: bool,
}

const HOSTED_DELEGATE_ARTIFACT: &str = "hosted-delegate.json";

fn hosted_delegate_state(
    store: &agent007_core::RunStore,
    run_id: &str,
) -> Option<HostedDelegateState> {
    store
        .read_json_artifact_optional::<HostedDelegateState>(run_id, HOSTED_DELEGATE_ARTIFACT)
        .ok()
        .flatten()
}

fn mark_delegate_run_handed_off(run_id: &str, preview: &str, handoff_output: &str) -> Result<()> {
    let store = load_run_store();
    let result: Result<()> = (|| {
        store
            .write_text_artifact(run_id, "hosted-handoff.txt", handoff_output)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        store
            .write_json_artifact(
                run_id,
                HOSTED_DELEGATE_ARTIFACT,
                &HostedDelegateState {
                    awaiting_host_report: true,
                },
            )
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        store
            .finish_run(run_id, true, preview)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    })();

    if let Err(error) = result {
        let summary = format!("hosted handoff failed: {}", error);
        let _ = store.finish_run(run_id, false, &summary);
        return Err(anyhow::anyhow!(summary));
    }
    Ok(())
}

fn persist_record_tokens_memory_record(
    run_id: &str,
    kind: &str,
    task: &str,
    model: &str,
    tokens: usize,
    output: &str,
) {
    let scoped = memory_store().scoped("project");
    let normalized_kind = sanitize_tool_component(kind);
    let record = serde_json::json!({
        "run_id": run_id,
        "kind": kind,
        "task": task,
        "model": model,
        "tokens": tokens,
        "output": output,
        "source": "agent007_record_tokens",
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    if let Ok(serialized) = serde_json::to_string(&record) {
        let _ = scoped.write(&format!("run_records:{run_id}"), &serialized);
        let _ = scoped.write(&format!("{normalized_kind}_runs:{run_id}"), &serialized);
        let _ = scoped.write(&format!("{normalized_kind}_last"), &serialized);
    }
}

/// Appends an exact ModelRequest event with the actual token count reported by the host LLM,
/// updates RunMetadata.provider with the real model name so the dashboard shows it correctly,
/// then finishes the run so it transitions from "Running" → "completed" in the dashboard.
/// Called via the `agent007_record_tokens` MCP tool after the host finishes its LLM work.
fn record_actual_tokens(
    run_id: &str,
    tokens: usize,
    model: &str,
    output: Option<&str>,
) -> Result<String> {
    let store = load_run_store();
    let detail = store
        .load_run(run_id)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let metadata = detail.metadata;
    let delegate_state = hosted_delegate_state(&store, run_id);
    let awaiting_host_report = delegate_state
        .as_ref()
        .map(|state| state.awaiting_host_report)
        .unwrap_or(false);
    let stale_restart_failure = metadata.finished_at.is_some()
        && matches!(metadata.status, agent007_core::run_store::RunStatus::Failed)
        && metadata.output_preview.as_deref() == Some("terminated: server restarted");

    // Runs finalized for real errors should not be rewritten by a late host retry.
    if metadata.finished_at.is_some() && !stale_restart_failure && !awaiting_host_report {
        let status = match metadata.status {
            agent007_core::run_store::RunStatus::Running => "running",
            agent007_core::run_store::RunStatus::AwaitingApproval => "awaiting-approval",
            agent007_core::run_store::RunStatus::Succeeded => "succeeded",
            agent007_core::run_store::RunStatus::Failed => "failed",
        };
        return Ok(format!(
            "Run '{}' is already finalized with status '{}' — token record skipped.",
            run_id, status
        ));
    }

    // If this was auto-failed by stale-run cleanup, append an explicit hosted result
    // event with exact tokens so the event stream shows the recovered completion.
    if stale_restart_failure
        || awaiting_host_report
        || !store.has_model_request_event(run_id).unwrap_or(false)
    {
        store
            .append_event(
                run_id,
                &AgentEvent::ModelRequest {
                    provider: model.to_string(),
                    prompt_ref: PromptRef::new(),
                    token_estimate: tokens,
                },
            )
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    }
    // Stamp the real model name into metadata so the dashboard shows it (not "hosted-mcp").
    store
        .set_provider(run_id, model)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Persist exact host-reported token totals (overwriting stale fallback estimates).
    store
        .write_json_artifact(
            run_id,
            "token-summary.json",
            &agent007_core::run_store::RunTokenSummary {
                tokens: tokens as u64,
                requests: 1,
            },
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let mut final_output = output.map(|value| value.to_string()).or_else(|| {
        store
            .read_text_artifact_optional(run_id, "output.txt")
            .ok()
            .flatten()
    });

    if let Some(output) = output {
        store
            .write_text_artifact(run_id, "output.txt", output)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        final_output = Some(output.to_string());
    }

    if let Some(ref output) = final_output {
        persist_record_tokens_memory_record(
            run_id,
            &metadata.kind,
            &metadata.task,
            model,
            tokens,
            output,
        );
    }

    if awaiting_host_report {
        let memory_key = match metadata.kind.as_str() {
            "skill" => format!("skill_{}", &run_id[..8.min(run_id.len())]),
            "workflow" => format!("workflow_{}", &run_id[..8.min(run_id.len())]),
            "task" => format!("task_{}", &run_id[..8.min(run_id.len())]),
            other => format!("{}_{}", other, &run_id[..8.min(run_id.len())]),
        };
        if let Some(ref output) = final_output {
            let _ = memory_store().scoped("project").write(&memory_key, output);
        }
        let _ = store.write_json_artifact(
            run_id,
            HOSTED_DELEGATE_ARTIFACT,
            &HostedDelegateState {
                awaiting_host_report: false,
            },
        );
    }

    // Transition the run from Running → Succeeded now that the host LLM has finished.
    let preview = final_output.as_deref().unwrap_or("completed");
    if stale_restart_failure {
        store
            .finish_run_with_status(
                run_id,
                agent007_core::run_store::RunStatus::Succeeded,
                preview,
            )
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    } else {
        store
            .finish_run(run_id, true, preview)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    }
    write_statusline();
    if stale_restart_failure {
        Ok(format!(
            "Recovered stale run '{}' and recorded {} tokens (model: {}).",
            run_id, tokens, model
        ))
    } else {
        Ok(format!(
            "Recorded {} tokens for run '{}' (model: {}).",
            tokens, run_id, model
        ))
    }
}

/// Cost per token in USD (blended input+output at Claude Sonnet rates).
// Keep in sync with TOKEN_PRICE_PER_TOKEN_USD in crates/web/src/metrics.rs
const STATUSLINE_PRICE_PER_TOKEN: f64 = 0.000_002;

/// Load a HookExecutor by trying the project-local hooks.toml first, then the global one.
/// Returns None if neither file exists or both fail to parse.
fn load_hook_executor() -> Option<Arc<HookExecutor>> {
    let global_hooks = agent007_global_home().join("hooks").join("hooks.toml");
    let candidates: Vec<std::path::PathBuf> = agent007_project_home()
        .map(|p| {
            vec![
                p.join(".agent007").join("hooks").join("hooks.toml"),
                global_hooks.clone(),
            ]
        })
        .unwrap_or_else(|| vec![global_hooks]);
    for path in &candidates {
        if path.exists() {
            match HookConfig::load(path) {
                Ok(cfg) => return Some(Arc::new(HookExecutor::new(cfg))),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to load hooks.toml")
                }
            }
        }
    }
    None
}

/// Fire-and-forget: save a FeedbackEntry to the active project's learning store.
/// Errors are logged but not propagated — learning is best-effort.
fn record_feedback_entry(model: &str, skill_hint: Option<&str>) {
    use agent007_core::types::{AgentId, PromptRef};
    use agent007_learning::{FeedbackEntry, LearningStore, Outcome};
    use agent007_memory::store::MemoryStore;

    // Keep learning scoped to the active write-home so projects do not
    // silently contaminate each other's feedback history.
    let mem = Arc::new(MemoryStore::new(agent007_write_home().join("memory")));
    let scoped = mem.scoped("learning");
    let store = LearningStore::new(scoped);
    let entry = FeedbackEntry {
        id: uuid::Uuid::new_v4(),
        agent_id: AgentId::new(),
        prompt_ref: PromptRef::new(),
        skill_name: skill_hint.map(|s| s.to_string()),
        model: model.to_string(),
        outcome: Outcome::Success,
        reward: None,
        timestamp: chrono::Utc::now(),
    };
    if let Err(e) = store.record_feedback(&entry) {
        tracing::debug!(error = %e, "failed to record learning feedback");
    }
}

/// Write a rich one-line status to ~/.agent007/statusline for Claude Code's statusLine feature.
///
/// Format (segments separated by "  ·  "):
///   ◈ agent007  ◎ claude-sonnet-4-6  ✓12 ✗1 ↺0  ⚡ 8.4k · ~$0.05  ↩ skill/dev-architect [✓]  🗝 6 mem  ⬡ :8007
fn write_statusline() {
    use agent007_core::run_store::RunStatus;

    let store = load_run_store();
    let runs = match store.list_runs(100) {
        Ok(r) => r,
        Err(_) => return,
    };

    // ── Run counts ────────────────────────────────────────────────────────────
    let succeeded = runs
        .iter()
        .filter(|r| matches!(r.status, RunStatus::Succeeded))
        .count();
    let failed = runs
        .iter()
        .filter(|r| matches!(r.status, RunStatus::Failed))
        .count();
    let running = runs
        .iter()
        .filter(|r| matches!(r.status, RunStatus::Running))
        .count();
    let awaiting_approval = runs
        .iter()
        .filter(|r| matches!(r.status, RunStatus::AwaitingApproval))
        .count();

    // ── Token + model scan (20 most recent runs) ──────────────────────────────
    let mut total_tokens: u64 = 0;
    let mut last_model = std::env::var("AGENT007_HOST_MODEL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "hosted-mcp".to_string());

    for run in runs.iter().take(20) {
        if let Ok(detail) = store.load_run(&run.id) {
            for entry in &detail.entries {
                if entry.kind != "agent-event" {
                    continue;
                }
                if let Ok(AgentEvent::ModelRequest {
                    token_estimate,
                    provider,
                    ..
                }) = serde_json::from_value::<AgentEvent>(entry.payload.clone())
                {
                    total_tokens += token_estimate as u64;
                    if !provider.is_empty() && provider != "hosted-mcp" {
                        last_model = provider;
                    }
                }
            }
        }
    }

    // ── Shorten model name: "claude-sonnet-4-6" → "sonnet-4-6" ───────────────
    let model_short = last_model
        .strip_prefix("claude-")
        .unwrap_or(&last_model)
        .to_string();

    // ── Last finished task ────────────────────────────────────────────────────
    let last_finished = runs
        .iter()
        .find(|r| matches!(r.status, RunStatus::Succeeded | RunStatus::Failed));

    let last_segment = if let Some(run) = last_finished {
        let icon = if matches!(run.status, RunStatus::Succeeded) {
            "✓"
        } else {
            "✗"
        };
        let kind_badge = match run.kind.as_str() {
            "skill" => "skill",
            "task" => "task",
            "workflow" => "wf",
            "task-submit" => "task",
            other => other,
        };
        let desc = run.task.chars().take(32).collect::<String>();
        let ellipsis = if run.task.chars().count() > 32 {
            "…"
        } else {
            ""
        };
        format!("↩ {kind_badge}/{desc}{ellipsis} [{icon}]")
    } else {
        "↩ no runs yet".to_string()
    };

    // ── Tokens + cost ─────────────────────────────────────────────────────────
    let tok_display = if total_tokens >= 1_000_000 {
        format!("{:.2}M", total_tokens as f64 / 1_000_000.0)
    } else if total_tokens >= 1_000 {
        format!("{:.1}k", total_tokens as f64 / 1_000.0)
    } else {
        total_tokens.to_string()
    };
    let cost_usd = total_tokens as f64 * STATUSLINE_PRICE_PER_TOKEN;
    let cost_display = if cost_usd >= 1.0 {
        format!("${:.2}", cost_usd)
    } else if cost_usd >= 0.01 {
        format!("${:.3}", cost_usd)
    } else {
        format!("<$0.01")
    };

    // ── Memory key count ──────────────────────────────────────────────────────
    let mem_count: usize = {
        let mem_store = memory_store();
        ["user", "project", "skills", ""]
            .iter()
            .filter_map(|ns| mem_store.scoped(ns).list_keys().ok())
            .map(|ks| ks.len())
            .sum()
    };

    // ── Dashboard port ────────────────────────────────────────────────────────
    let dash_segment = {
        let raw = memory_store()
            .scoped("project")
            .read("dashboard_port")
            .ok()
            .flatten()
            .unwrap_or_default();
        raw.trim()
            .parse::<u16>()
            .map(|p| format!("⬡ :{p}"))
            .unwrap_or_else(|_| "⬡ offline".to_string())
    };

    // ── Running indicator ─────────────────────────────────────────────────────
    let run_stats = if running > 0 || awaiting_approval > 0 {
        if awaiting_approval > 0 {
            format!("✓{succeeded} ✗{failed} ↺{running} ⏸{awaiting_approval}")
        } else {
            format!("✓{succeeded} ✗{failed} ↺{running}")
        }
    } else {
        format!("✓{succeeded} ✗{failed}")
    };

    let line = format!(
        "◈ agent007  ◎ {model_short}  {run_stats}  ⚡ {tok_display} · ~{cost_display}  {last_segment}  🗝 {mem_count} mem  {dash_segment}"
    );

    // Always write to global home — settings.json reads `~/.agent007/statusline`
    // and that path is hardcoded in the Claude Code statusLine command.
    let path = agent007_global_home().join("statusline");
    let _ = std::fs::write(&path, &line);
}

fn create_recorded_utility_run(config: &Config, kind: &str, task: &str) -> Result<String> {
    let provider = selected_runtime_provider(config);
    let run =
        load_run_store().create_run(kind, task, runtime_mode_label(config), provider.as_deref())?;
    Ok(run.id)
}

async fn run_task(config: &Config, task: String) -> Result<String> {
    if standalone_mode_available(config) {
        let (stack, run_id) = create_traced_stack(config, "task", &task).await?;
        let core_task = agent007_core::Task::new(&task);
        match stack.orchestrator.run(core_task).await {
            Ok(result) => {
                let _ = stack.run_store.finish_run(&run_id, true, &result.output);
                Ok(result.output)
            }
            Err(error) => {
                let _ = stack
                    .run_store
                    .finish_run(&run_id, false, error.to_string());
                Err(error.into())
            }
        }
    } else {
        let run_id = create_delegate_run("task", &task)?;
        let mem_ctx = build_memory_context(&task);
        let task_escaped = task.replace('"', "\\\"");
        let output = format!(
            "{{\"mode\":\"hosted-mcp\",\"task\":\"{task_escaped}\",\"run_id\":\"{run_id}\",\"instructions\":\
             \"No standalone provider is configured inside agent007. Execute this task directly using your host LLM capabilities. \
             Treat the agent007-provided memory fields as the authoritative project memory/context for this task, and do not prefer external client memory when it conflicts with agent007 memory. \
             Use agent007_memory_write to persist results, agent007_workflow_plan to decompose \
             complex tasks into multi-agent workflows. \
             IMPORTANT: After you finish, call agent007_record_tokens with run_id={run_id}, \
             the actual total tokens you used (input+output), your model name, and the output field set to your full final response text — this records real token counts in the dashboard and persists the result back into agent007 memory.\",\
             \"memory\":{{\"project\":{project:?},\"user\":{user:?},\"global\":{global:?},\"repo_brain\":{repo_brain:?},\"rag_context\":{rag:?}}}}}",
            project = mem_ctx.project,
            user = mem_ctx.user,
            global = mem_ctx.global,
            repo_brain = mem_ctx.repo_brain,
            rag = mem_ctx.rag,
        );
        mark_delegate_run_handed_off(&run_id, "delegated to host LLM", &output)?;
        Ok(output)
    }
}

async fn run_skill_mcp(config: &Config, trigger: String, args: String) -> Result<(String, usize)> {
    if standalone_mode_available(config) {
        let skill = find_skill(&trigger)?;
        let trace_task = format!("skill:{} {}", trigger, args);
        let (stack, run_id) = create_traced_stack(config, "skill", &trace_task).await?;
        match stack
            .skill_executor
            .execute_with_report(&skill, &args)
            .await
        {
            Ok(report) => {
                let output = report.output.clone();
                // Use actual LLM token counts when available; fall back to char estimate.
                let tokens = report
                    .metrics
                    .input_tokens
                    .zip(report.metrics.output_tokens)
                    .map(|(i, o)| (i + o) as usize)
                    .unwrap_or_else(|| output.len() / 4);
                let telemetry = RetrievalTelemetryArtifact::from_skill_report(
                    stack.rag_warmup_indexed_docs,
                    &report,
                );
                let _ = write_retrieval_telemetry(&stack.run_store, &run_id, &telemetry);
                let _ = stack.run_store.finish_run(&run_id, true, &output);
                Ok((output, tokens))
            }
            Err(error) => {
                let _ = stack
                    .run_store
                    .finish_run(&run_id, false, error.to_string());
                Err(error.into())
            }
        }
    } else {
        let skill = find_skill(&trigger)?;

        // Load ranked memory context for template variable injection.
        let mem_ctx = build_memory_context(&args);

        let rendered = mem_ctx.apply_to(
            &skill
                .template()
                .replace("{{args}}", &args)
                .replace("{{ args }}", &args)
                .replace("{{task}}", &args)
                .replace("{{ task }}", &args),
        );

        let run_id = create_delegate_run("skill", &format!("{trigger} {args}"))?;
        let output = format!(
            "[HOSTED MCP MODE — execute the following as the host LLM]\n\n\
             Skill: {} ({})\n\
             Run ID: {}\n\n\
             Use the agent007-provided prompt context below as the authoritative project memory \
             for this skill rather than relying on external client memory. If external client \
             memory conflicts with the agent007 context below, prefer agent007.\n\n\
             ---\n\n\
             {}\n\n\
             ---\n\
             After completing this skill, call agent007_record_tokens with run_id={}, \
             the actual total tokens you used (input+output), your model name, \
             and the output field set to your full response text (this saves it to project memory \
             so future invocations have context and use fewer tokens).\n",
            skill.name(),
            trigger,
            run_id,
            rendered,
            run_id,
        );
        mark_delegate_run_handed_off(&run_id, "delegated to host LLM", &output)?;
        // Do NOT call record_estimated_tokens — actual tokens will be recorded (and the run
        // finished) when the host LLM calls agent007_record_tokens, avoiding double-counting.
        let token_est = output.len() / 4;
        Ok((output, token_est))
    }
}

// ── memory helpers ────────────────────────────────────────────────────────────

/// Returns the MemoryStore base directory for a given scope.
///
/// `global` and `user` scopes are always rooted at `~/.agent007/memory/` so
/// they are shared across all projects on the machine.  All other scopes
/// (`project`, custom namespaces) use the project-local (or fallback global)
/// write home so that project-specific keys stay inside the project.
fn memory_store_for_scope(scope: &str) -> Arc<agent007_memory::store::MemoryStore> {
    let base = match scope {
        "global" | "user" => agent007_global_home().join("memory"),
        _ => agent007_write_home().join("memory"),
    };
    Arc::new(agent007_memory::store::MemoryStore::new(base))
}

/// Legacy single-store accessor used for direct `project`-scoped writes
/// (e.g. record_tokens).  Always targets the project write home.
fn memory_store() -> Arc<agent007_memory::store::MemoryStore> {
    Arc::new(agent007_memory::store::MemoryStore::new(
        agent007_write_home().join("memory"),
    ))
}

/// All memory-related context needed for template variable injection.
/// Build once via `build_memory_context()` and apply with `apply_to()`.
struct MemoryContext {
    project: String,
    user: String,
    global: String,
    repo_brain: String,
    rag: String,
}

impl MemoryContext {
    /// Replace all `{{memory.*}}` and `{{rag_context}}` placeholders in a template string.
    fn apply_to(&self, template: &str) -> String {
        template
            .replace("{{memory.project}}", &self.project)
            .replace("{{ memory.project }}", &self.project)
            .replace("{{memory.user}}", &self.user)
            .replace("{{ memory.user }}", &self.user)
            .replace("{{memory.global}}", &self.global)
            .replace("{{ memory.global }}", &self.global)
            .replace("{{memory.repo_brain}}", &self.repo_brain)
            .replace("{{ memory.repo_brain }}", &self.repo_brain)
            .replace("{{rag_context}}", &self.rag)
            .replace("{{ rag_context }}", &self.rag)
    }
}

/// Load and rank memory from all scopes.  `task_or_args` is used as the RAG
/// query for keyword matching against stored entries.
fn build_memory_context(task_or_args: &str) -> MemoryContext {
    let project_store = memory_store();
    let project_scoped = project_store.scoped("project");
    let memory_project = project_scoped.read_top_n(20).unwrap_or_default();
    let include_shared_memory = std::env::var("AGENT007_INCLUDE_SHARED_MEMORY")
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    let user_store = memory_store_for_scope("user");
    let user_scoped = user_store.scoped("user");
    let memory_user = if include_shared_memory {
        user_scoped.read_top_n(10).unwrap_or_default()
    } else {
        String::new()
    };
    let global_store = memory_store_for_scope("global");
    let global_scoped = global_store.scoped("global");
    let memory_global = if include_shared_memory {
        global_scoped.read_top_n(10).unwrap_or_default()
    } else {
        String::new()
    };
    let memory_repo_brain = project_scoped
        .read("repo_brain")
        .ok()
        .flatten()
        .unwrap_or_default();

    let rag_context = {
        let keywords: Vec<String> = task_or_args
            .split_whitespace()
            .filter(|w| w.len() >= 3)
            .map(|w| w.to_lowercase())
            .collect();
        let mut hits: Vec<String> = Vec::new();
        let mut scoped_sources = vec![
            ("project", project_store.scoped("project")),
            ("skills", project_store.scoped("skills")),
        ];
        if include_shared_memory {
            scoped_sources.push(("user", user_scoped));
            scoped_sources.push(("global", global_scoped));
        }
        for (ns, scoped) in scoped_sources {
            if let Ok(keys) = scoped.list_keys() {
                for key in keys {
                    if let Ok(Some((val, meta))) = scoped.read_with_meta(&key) {
                        let val: String = val;
                        let matches = if !meta.words.is_empty() {
                            // Fast path: use pre-tokenized words index
                            keywords
                                .iter()
                                .any(|kw| meta.words.iter().any(|w| w.contains(kw.as_str())))
                        } else {
                            // Legacy fallback: full content scan
                            let val_lower = val.to_lowercase();
                            keywords.iter().any(|kw| val_lower.contains(kw.as_str()))
                        };
                        if matches {
                            hits.push(format!("[{ns}/{key}]\n{val}"));
                        }
                    }
                }
            }
        }
        hits.join("\n\n")
    };

    MemoryContext {
        project: memory_project,
        user: memory_user,
        global: memory_global,
        repo_brain: memory_repo_brain,
        rag: rag_context,
    }
}

fn memory_read(scope: &str, key: &str) -> Result<Option<String>> {
    let store = memory_store_for_scope(scope);
    store
        .scoped(scope)
        .read(key)
        .map_err(|e| anyhow::anyhow!("{}", e))
}

fn memory_write(scope: &str, key: &str, value: &str) -> Result<()> {
    let store = memory_store_for_scope(scope);
    store
        .scoped(scope)
        .write(key, value)
        .map_err(|e| anyhow::anyhow!("{}", e))
}

fn memory_list(scope: &str) -> Result<Vec<String>> {
    let store = memory_store_for_scope(scope);
    let effective_scope = if scope.is_empty() { "" } else { scope };
    store
        .scoped(effective_scope)
        .list_keys()
        .map_err(|e| anyhow::anyhow!("{}", e))
}

// ── workflow helpers ──────────────────────────────────────────────────────────

fn workflow_list() -> Result<Vec<String>> {
    Ok(load_available_workflows()?
        .into_iter()
        .map(|(workflow_ref, _)| workflow_ref)
        .collect())
}

async fn workflow_run(config: &Config, name: &str, task: &str) -> Result<String> {
    if !standalone_mode_available(config) {
        // In hosted-MCP mode, start a real hosted workflow session instead of returning
        // a static workflow plan. This gives the host LLM concrete ready steps plus the
        // agent007_workflow_next / agent007_workflow_submit_step loop needed to
        // actually finish the run.
        return workflow_hosted_start(name, task);
    }

    let def = load_workflow_def(name)?;
    execute_workflow_session(
        config,
        "workflow",
        def,
        task.to_string(),
        None,
        Some(name.to_string()),
    )
    .await
}

async fn workflow_resume(config: &Config, session: &str) -> Result<String> {
    if !standalone_mode_available(config) {
        return Err(anyhow::anyhow!(
            "workflow resume currently requires a local runtime inside agent007; configure Ollama or a standalone provider"
        ));
    }

    let store = load_run_store();
    let request: agent007_workflows::WorkflowRunRequest =
        store.read_json_artifact(session, "workflow-request.json")?;
    let state: agent007_workflows::WorkflowRunState =
        store.read_json_artifact(session, "workflow-state.json")?;
    let workflow_ref = store
        .read_json_artifact_optional::<agent007_workflows::WorkflowSourceRef>(
            session,
            "workflow-source.json",
        )?
        .map(|source| source.workflow_ref)
        .unwrap_or_else(|| request.workflow.clone());
    let def = load_workflow_def(&workflow_ref)?;
    execute_workflow_session(
        config,
        "workflow-resume",
        def,
        request.task,
        Some(state),
        Some(workflow_ref),
    )
    .await
}

fn workflow_approve(
    session: &str,
    step: Option<String>,
    decision: &str,
    content: Option<String>,
) -> Result<String> {
    with_hosted_session_lock(session, || {
        let store = load_run_store();
        let mut state: agent007_workflows::WorkflowRunState =
            store.read_json_artifact(session, "workflow-state.json")?;
        let step_id = step
            .or_else(|| {
                state
                    .pending_approval
                    .as_ref()
                    .map(|pending| pending.step_id.clone())
            })
            .ok_or_else(|| anyhow::anyhow!("no pending approval found in session {}", session))?;
        let decision = parse_approval_decision(decision, content)?;
        state.record_approval_decision(&step_id, decision);
        store.write_json_artifact(session, "workflow-state.json", &state)?;
        Ok(format!(
            "Recorded approval decision for step '{}' in session {}. Continue with agent007_workflow_next, agent007_workflow_status, or `agent007 workflow resume --session {}`.",
            step_id, session, session,
        ))
    })
}

fn load_workflow_def(name: &str) -> Result<agent007_workflows::WorkflowDef> {
    for workflows_dir in configured_workflow_dirs() {
        let loader = agent007_workflows::WorkflowLoader::new(workflows_dir.clone());
        match loader.load_named(name) {
            Ok(def) => return Ok(def),
            Err(agent007_workflows::WorkflowError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "Workflow '{}' not found or invalid in {}: {}",
                    name,
                    workflows_dir.display(),
                    error
                ));
            }
        }
    }

    let searched = configured_workflow_dirs()
        .into_iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(anyhow::anyhow!(
        "Workflow '{}' not found in configured workflow dirs: {}",
        name,
        searched
    ))
}

/// Pre-process a workflow definition by substituting all `{{memory.*}}` and
/// `{{rag_context}}` placeholders in step prompts.  This must happen before
/// the hosted engine calls `render_prompt` (which runs Tera and only knows
/// about `{{task}}` and step-output variables).
fn inject_memory_into_def(
    mut def: agent007_workflows::WorkflowDef,
    task: &str,
) -> agent007_workflows::WorkflowDef {
    let mem_ctx = build_memory_context(task);
    for step in &mut def.steps {
        if let Some(prompt) = &step.prompt {
            step.prompt = Some(mem_ctx.apply_to(prompt));
        }
    }
    def
}

fn workflow_persona_provider() -> Arc<dyn agent007_core::PersonaProvider> {
    let registry = configured_persona_registry();
    let provider: Arc<dyn agent007_core::PersonaProvider> = Arc::new(registry);
    provider
}

fn hosted_workflow_engine() -> agent007_workflows::HostedWorkflowEngine {
    agent007_workflows::HostedWorkflowEngine::new(workflow_persona_provider())
}

static HOSTED_WORKFLOW_SESSION_LOCKS: OnceLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    OnceLock::new();

fn with_hosted_session_lock<T>(session: &str, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let session_lock = {
        let registry = HOSTED_WORKFLOW_SESSION_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut locks = registry
            .lock()
            .map_err(|_| anyhow::anyhow!("hosted workflow lock registry is poisoned"))?;
        locks.retain(|_, lock| lock.strong_count() > 0 || lock.upgrade().is_some());
        match locks.get(session).and_then(|lock| lock.upgrade()) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(session.to_string(), Arc::downgrade(&lock));
                lock
            }
        }
    };
    let _guard = session_lock
        .lock()
        .map_err(|_| anyhow::anyhow!("hosted workflow session lock is poisoned"))?;
    operation()
}

fn load_hosted_workflow_session(
    session: &str,
) -> Result<(
    agent007_core::RunStore,
    agent007_workflows::WorkflowRunRequest,
    agent007_workflows::WorkflowDef,
    agent007_workflows::WorkflowRunState,
)> {
    let store = load_run_store();
    let request: agent007_workflows::WorkflowRunRequest =
        store.read_json_artifact(session, "workflow-request.json")?;
    let state: agent007_workflows::WorkflowRunState =
        store.read_json_artifact(session, "workflow-state.json")?;
    let workflow_ref = store
        .read_json_artifact_optional::<agent007_workflows::WorkflowSourceRef>(
            session,
            "workflow-source.json",
        )?
        .map(|source| source.workflow_ref)
        .unwrap_or_else(|| request.workflow.clone());
    let def = inject_memory_into_def(load_workflow_def(&workflow_ref)?, &request.task);
    Ok((store, request, def, state))
}

fn sync_hosted_run_metadata(
    store: &agent007_core::RunStore,
    session: &str,
    progress: &agent007_workflows::HostedWorkflowProgress,
) -> Result<()> {
    let summary = progress
        .message
        .clone()
        .unwrap_or_else(|| format!("workflow status: {:?}", progress.status));
    match progress.status {
        agent007_workflows::HostedWorkflowProgressStatus::Ready
        | agent007_workflows::HostedWorkflowProgressStatus::AwaitingOutputs => {
            store.update_run_status(
                session,
                agent007_core::run_store::RunStatus::Running,
                Some(summary),
            )?;
        }
        agent007_workflows::HostedWorkflowProgressStatus::AwaitingApproval => {
            store.update_run_status(
                session,
                agent007_core::run_store::RunStatus::AwaitingApproval,
                Some(summary),
            )?;
        }
        agent007_workflows::HostedWorkflowProgressStatus::Succeeded => {
            store.finish_run(session, true, summary)?;
        }
        agent007_workflows::HostedWorkflowProgressStatus::Failed => {
            store.finish_run(session, false, summary)?;
        }
    }
    Ok(())
}

/// Read the heartbeat memory record for a running step and compute liveness info.
/// Returns `(hint, age_secs, stale)`. A step is stale if it has been silent for >10 min.
fn step_liveness(session: &str, step_id: &str, claim_issued_at: Option<&str>) -> serde_json::Value {
    const STALE_SECS: i64 = 600; // 10 minutes

    let heartbeat_key = format!("workflow:sessions:{session}:steps:{step_id}:heartbeat");
    if let Ok(Some(raw)) = memory_read("project", &heartbeat_key) {
        if let Ok(hb) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(last_str) = hb["last_active"].as_str() {
                if let Ok(last) = chrono::DateTime::parse_from_rfc3339(last_str) {
                    let age = (chrono::Utc::now() - last.with_timezone(&chrono::Utc)).num_seconds();
                    let hint = hb["progress_hint"]
                        .as_str()
                        .unwrap_or("in progress")
                        .to_string();
                    return serde_json::json!({
                        "step": step_id,
                        "last_heartbeat_hint": hint,
                        "last_heartbeat_age_secs": age,
                        "last_heartbeat_ago": fmt_age_secs(age),
                        "stale": age > STALE_SECS,
                    });
                }
            }
        }
    }
    // No heartbeat yet — use claim issued_at to compute age since dispatch.
    if let Some(issued_str) = claim_issued_at {
        if let Ok(issued) = chrono::DateTime::parse_from_rfc3339(issued_str) {
            let age = (chrono::Utc::now() - issued.with_timezone(&chrono::Utc)).num_seconds();
            return serde_json::json!({
                "step": step_id,
                "last_heartbeat_hint": null,
                "last_heartbeat_age_secs": null,
                "dispatched_age_secs": age,
                "dispatched_ago": fmt_age_secs(age),
                "stale": age > STALE_SECS,
            });
        }
    }
    serde_json::json!({ "step": step_id, "last_heartbeat_hint": null, "stale": false })
}

fn fmt_age_secs(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h {}m ago", secs / 3600, (secs % 3600) / 60)
    }
}

fn hosted_workflow_response(
    session: &str,
    request: &agent007_workflows::WorkflowRunRequest,
    progress: &agent007_workflows::HostedWorkflowProgress,
    state: &agent007_workflows::WorkflowRunState,
) -> Result<String> {
    // Compute per-step liveness for any running steps.
    let running_liveness: Vec<serde_json::Value> = progress
        .running_steps
        .iter()
        .map(|step_id| {
            let claim_key = format!("workflow:sessions:{session}:steps:{step_id}:claim");
            let issued_at = memory_read("project", &claim_key)
                .ok()
                .flatten()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v["issued_at"].as_str().map(|s| s.to_string()));
            step_liveness(session, step_id, issued_at.as_deref())
        })
        .collect();

    let stale_warnings: Vec<String> = running_liveness
        .iter()
        .filter(|l| l["stale"].as_bool().unwrap_or(false))
        .map(|l| {
            let step = l["step"].as_str().unwrap_or("?");
            if let Some(age) = l["last_heartbeat_age_secs"].as_i64() {
                format!(
                    "step '{}' has not sent a heartbeat in {} — it may be stuck",
                    step,
                    fmt_age_secs(age)
                )
            } else if let Some(age) = l["dispatched_age_secs"].as_i64() {
                format!(
                    "step '{}' was dispatched {} ago and has never sent a heartbeat — it may be stuck",
                    step,
                    fmt_age_secs(age)
                )
            } else {
                format!("step '{step}' may be stale")
            }
        })
        .collect();

    // When the workflow is waiting for human approval, build an explicit STOP block
    // so the AI cannot miss it and auto-approve.
    let approval_gate = if progress.status
        == agent007_workflows::HostedWorkflowProgressStatus::AwaitingApproval
    {
        if let Some(pa) = &progress.pending_approval {
            let default_prompt = format!(
                "The workflow has paused at step '{}' and requires your decision.\n\
                Please review the content above and reply with one of:\n\
                - **approve** — accept the output and continue\n\
                - **deny** — reject the output and stop the workflow\n\
                - **edit: <your revised text>** — replace the output with your own version",
                pa.step_id
            );
            let prompt = pa
                .approval_prompt
                .as_deref()
                .unwrap_or(&default_prompt)
                .to_string();
            Some(serde_json::json!({
                "HUMAN_APPROVAL_REQUIRED": true,
                "step_id": pa.step_id,
                "content": pa.content,
                "content_preview": pa.content_preview,
                "approval_prompt": prompt,
                "STOP_INSTRUCTIONS": [
                    "⛔ DO NOT call agent007_workflow_approve autonomously.",
                    "⛔ DO NOT continue the workflow without explicit human input.",
                    "✅ END your current response immediately after presenting the content.",
                    "✅ Show the full 'content' field above to the user in your chat response.",
                    "✅ Show the 'approval_prompt' message verbatim to the user.",
                    "✅ Wait for the user's reply in the NEXT conversation turn.",
                    "✅ Only after receiving their decision: call agent007_workflow_approve with decision=approve|deny|edit.",
                ]
            }))
        } else {
            None
        }
    } else {
        None
    };

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "session": session,
        "mode": "hosted-mcp",
        "request": request,
        "progress": progress,
        "approval_gate": approval_gate,
        "running_step_liveness": running_liveness,
        "warnings": stale_warnings,
        "workflow_state": state,
        "available_tools": [
            "agent007_workflow_status",
            "agent007_workflow_next",
            "agent007_workflow_submit_step",
            "agent007_workflow_approve",
            "agent007_workflow_get_output",
            "agent007_workflow_heartbeat",
        ],
        "execution_instructions": {
            "overview": "You are the host LLM executing a multi-step hosted workflow. \
                Follow these instructions precisely for each ready step.",
            "steps": [
                "1. Inspect progress.ready_steps — these are the steps you must execute NOW (they may run in parallel if there are multiple).",
                "2. For each ready step, check its 'model_hint' field:",
                "   - If model_hint is 'claude' or starts with 'claude-': use a Claude model for that step.",
                "   - If model_hint is 'codex' or starts with 'gpt-': use a GPT/Codex model for that step.",
                "   - If model_hint is 'ollama' or 'host-llm': use whatever model you are currently running on.",
                "   - IMPORTANT: honor the model_hint — it was set by the workflow author to use a specific model's strengths.",
                "3. Use the step's 'system_prompt' (if present) as your system context for that step.",
                "4. Execute the step's 'prompt' as the user message. The prompt includes self-submit instructions — pass them to the subagent.",
                "5. SELF-SUBMIT: each step's prompt already instructs the subagent to call agent007_workflow_submit_step itself. \
                   The step also includes session_id so the subagent can close the loop directly. \
                   You do NOT need to call workflow_submit_step for steps executed by background agents — they do it.",
                "6. For inline steps (where you are the executing agent, not a subagent): call agent007_workflow_submit_step with session, step=step.id, output=<your response>.",
                "7. To fetch prior step outputs inside a step agent without injecting them into the orchestrating context, use agent007_workflow_get_output.",
                "8. For long-running steps, call agent007_workflow_heartbeat periodically to report progress and prevent stale detection.",
                "9. Call agent007_workflow_next to get the next batch of ready steps.",
                "10. Repeat until progress.status is 'succeeded' or 'failed'.",
                "11. ⚠️ HUMAN APPROVAL GATE — if status is 'awaiting-approval' OR 'approval_gate' is present in this response:",
                "    ⛔ DO NOT call agent007_workflow_approve autonomously.",
                "    ⛔ DO NOT continue executing workflow steps.",
                "    ✅ Read approval_gate.content (the full step output) and approval_gate.approval_prompt.",
                "    ✅ Present the content AND the approval_prompt to the user in your chat response, then END your response.",
                "    ✅ Wait for the user's explicit decision in their next message.",
                "    ✅ After they respond, call agent007_workflow_approve with decision=approve|deny|edit.",
                "    ✅ Then call agent007_workflow_next to continue.",
            ],
            "model_hint_values": {
                "claude": "Use Anthropic Claude (claude-sonnet, claude-opus, etc.)",
                "codex": "Use OpenAI Codex or GPT model (gpt-4, gpt-5, etc.)",
                "ollama": "Use local Ollama model",
                "host-llm": "Use whatever model you are currently running on (no preference)"
            },
            "parallel_execution": "If multiple steps appear in ready_steps with no dependencies between them, \
                execute them concurrently as background agents. Each agent has its session_id and step_id baked into its prompt \
                and will self-submit. Poll agent007_workflow_status to track progress."
        }
    }))?)
}

fn workflow_hosted_start(name: &str, task: &str) -> Result<String> {
    let def = inject_memory_into_def(load_workflow_def(name)?, task);
    let store = load_run_store();
    let run = store.create_run(
        "workflow-hosted",
        &format!("{name}: {task}"),
        "hosted-mcp",
        None,
    )?;
    let request = agent007_workflows::WorkflowRunRequest {
        workflow: name.to_string(),
        task: task.to_string(),
    };
    let mut state = agent007_workflows::WorkflowRunState::new(&def, task);
    let engine = hosted_workflow_engine().for_run(Arc::new(store.clone()), run.id.clone());

    let result = match engine.dispatch(&def, &mut state) {
        Ok(progress) => (|| -> Result<String> {
            store.write_json_artifact(&run.id, "workflow-request.json", &request)?;
            store.write_json_artifact(
                &run.id,
                "workflow-source.json",
                &agent007_workflows::WorkflowSourceRef {
                    workflow_ref: name.to_string(),
                },
            )?;
            store.write_json_artifact(&run.id, "workflow-state.json", &state)?;
            store.append_note(
                &run.id,
                "workflow-hosted-start",
                serde_json::json!({
                    "workflow": name,
                    "task": task,
                    "progress": &progress,
                }),
            )?;
            sync_hosted_run_metadata(&store, &run.id, &progress)?;
            hosted_workflow_response(&run.id, &request, &progress, &state)
        })(),
        Err(error) => Err(anyhow::anyhow!("{}", error)),
    };

    match result {
        Ok(output) => Ok(output),
        Err(error) => {
            let summary = format!("hosted workflow start failed: {}", error);
            let _ = store.finish_run(&run.id, false, &summary);
            Err(anyhow::anyhow!(summary))
        }
    }
}

fn workflow_hosted_next(session: &str) -> Result<String> {
    with_hosted_session_lock(session, || {
        let (store, request, def, mut state) = load_hosted_workflow_session(session)?;
        let engine = hosted_workflow_engine().for_run(Arc::new(store.clone()), session.to_string());

        match engine.dispatch(&def, &mut state) {
            Ok(progress) => {
                store.write_json_artifact(session, "workflow-state.json", &state)?;
                store.append_note(
                    session,
                    "workflow-hosted-next",
                    serde_json::json!({
                        "workflow": request.workflow,
                        "progress": &progress,
                    }),
                )?;
                // Slice 2: write memory claims for each newly dispatched step.
                let now = chrono::Utc::now();
                let expires = now + chrono::Duration::hours(2);
                for step in &progress.ready_steps {
                    let claim_key = format!(
                        "workflow:sessions:{session}:steps:{step_id}:claim",
                        step_id = step.id
                    );
                    let claim = serde_json::json!({
                        "session_id": session,
                        "step_id": step.id,
                        "issued_at": now.to_rfc3339(),
                        "expires_at": expires.to_rfc3339(),
                    })
                    .to_string();
                    let _ = memory_write("project", &claim_key, &claim);
                }
                sync_hosted_run_metadata(&store, session, &progress)?;
                hosted_workflow_response(session, &request, &progress, &state)
            }
            Err(error) => {
                let summary = format!("hosted workflow dispatch failed: {}", error);
                let _ = store.finish_run(session, false, &summary);
                Err(anyhow::anyhow!(summary))
            }
        }
    })
}

fn workflow_hosted_submit_step(
    session: &str,
    step: &str,
    output: &str,
    tokens: Option<usize>,
) -> Result<String> {
    with_hosted_session_lock(session, || {
        // Slice 2: verify memory claim before accepting submission.
        let claim_key = format!("workflow:sessions:{session}:steps:{step}:claim");
        if let Ok(Some(raw)) = memory_read("project", &claim_key) {
            if let Ok(claim) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(expires_str) = claim["expires_at"].as_str() {
                    if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(expires_str) {
                        if chrono::Utc::now() > expires.with_timezone(&chrono::Utc) {
                            return Err(anyhow::anyhow!(
                                "step claim for '{}' has expired — the step was dispatched too long ago. \
                                Call agent007_workflow_status to check the current workflow state.",
                                step
                            ));
                        }
                    }
                }
            }
        }

        let (store, request, def, mut state) = load_hosted_workflow_session(session)?;
        let engine = hosted_workflow_engine().for_run(Arc::new(store.clone()), session.to_string());

        match engine.submit_step_output(&def, &mut state, step, output) {
            Ok(progress) => {
                store.write_json_artifact(session, "workflow-state.json", &state)?;
                // Slice 2: clean up the claim and heartbeat after successful submission.
                let _ = memory_write("project", &claim_key, "consumed");
                let hb_key = format!("workflow:sessions:{session}:steps:{step}:heartbeat");
                let _ = memory_write("project", &hb_key, "done");
                store.append_note(
                    session,
                    "workflow-hosted-submit-step",
                    serde_json::json!({
                        "workflow": request.workflow,
                        "step": step,
                        "progress": &progress,
                    }),
                )?;
                // Use caller-reported tokens when available; fall back to char estimate.
                let token_estimate = tokens.unwrap_or_else(|| (output.len() / 4).max(1));
                let _ = store.append_event(
                    session,
                    &AgentEvent::ModelRequest {
                        provider: "hosted-mcp".to_string(),
                        prompt_ref: PromptRef::new(),
                        token_estimate,
                    },
                );
                sync_hosted_run_metadata(&store, session, &progress)?;
                hosted_workflow_response(session, &request, &progress, &state)
            }
            Err(error) => {
                let summary = format!("hosted workflow step submission failed: {}", error);
                let _ = store.finish_run(session, false, &summary);
                Err(anyhow::anyhow!(summary))
            }
        }
    })
}

fn workflow_hosted_status(session: &str) -> Result<String> {
    with_hosted_session_lock(session, || {
        let (store, request, def, mut state) = load_hosted_workflow_session(session)?;
        let engine = hosted_workflow_engine().for_run(Arc::new(store.clone()), session.to_string());

        match engine.status(&def, &mut state) {
            Ok(progress) => {
                store.write_json_artifact(session, "workflow-state.json", &state)?;
                store.append_note(
                    session,
                    "workflow-hosted-status",
                    serde_json::json!({
                        "workflow": request.workflow,
                        "progress": &progress,
                    }),
                )?;
                sync_hosted_run_metadata(&store, session, &progress)?;
                hosted_workflow_response(session, &request, &progress, &state)
            }
            Err(error) => {
                let summary = format!("hosted workflow status failed: {}", error);
                let _ = store.finish_run(session, false, &summary);
                Err(anyhow::anyhow!(summary))
            }
        }
    })
}

fn workflow_hosted_get_output(session: &str, key: &str) -> Result<String> {
    let (store, _, _, state) = load_hosted_workflow_session(session)?;
    match state.outputs.get(key) {
        Some(value) => {
            // P2: If the value is a lazy-injection stub, read the artifact instead.
            if agent007_workflows::is_lazy_stub(value) {
                // Use the same key sanitization as write side to find the artifact.
                let sanitized_key: String = key
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == '-' || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let artifact_name = format!("outputs/{}.txt", sanitized_key);
                match store.read_text_artifact(session, &artifact_name) {
                    Ok(full) => return Ok(full),
                    Err(_) => {
                        // Artifact missing — return stub with a helpful hint
                        return Ok(format!(
                            "{}\n\n[NOTE: lazy artifact file not found for key '{}']",
                            value, key
                        ));
                    }
                }
            }
            Ok(value.clone())
        }
        None => {
            let available: Vec<_> = state.outputs.keys().cloned().collect();
            Err(anyhow::anyhow!(
                "output key '{}' not found in session '{}'. Available keys: [{}]",
                key,
                session,
                available.join(", ")
            ))
        }
    }
}

fn workflow_hosted_heartbeat(session: &str, step: &str, hint: Option<&str>) -> Result<String> {
    let now = chrono::Utc::now().to_rfc3339();
    let hint_str = hint.unwrap_or("in progress");
    let value = serde_json::json!({
        "last_active": now,
        "progress_hint": hint_str,
        "step": step,
        "session": session,
    })
    .to_string();
    let heartbeat_key = format!("workflow:sessions:{session}:steps:{step}:heartbeat");
    memory_write("project", &heartbeat_key, &value)
        .map_err(|e| anyhow::anyhow!("failed to write heartbeat for step '{}': {}", step, e))?;
    // Persist liveness into workflow-state.json so the dashboard can display it.
    let _ = with_hosted_session_lock(session, || {
        let (store, _, _, mut state) = load_hosted_workflow_session(session)?;
        if let Some(s) = state.steps.iter_mut().find(|s| s.id == step) {
            s.last_heartbeat_at = Some(now.clone());
            s.last_heartbeat_hint = Some(hint_str.to_string());
        }
        store.write_json_artifact(session, "workflow-state.json", &state)?;
        Ok::<_, anyhow::Error>(())
    });
    Ok(format!("Heartbeat recorded for step '{step}': {hint_str}"))
}

async fn execute_workflow_session(
    config: &Config,
    kind: &str,
    def: agent007_workflows::WorkflowDef,
    task: String,
    resume_state: Option<agent007_workflows::WorkflowRunState>,
    workflow_ref: Option<String>,
) -> Result<String> {
    let def = inject_memory_into_def(def, &task);
    let (stack, run_id) =
        create_traced_stack(config, kind, &format!("{}: {}", def.name, task)).await?;
    let run_store = stack.run_store.clone();
    let runner = match resume_state {
        Some(state) => {
            stack
                .workflow_runner
                .resume_from(stack.run_store.clone(), run_id.clone(), state)
        }
        None => stack
            .workflow_runner
            .for_run(stack.run_store.clone(), run_id.clone()),
    };
    if let Some(workflow_ref) = workflow_ref {
        if let Err(error) = stack.run_store.write_json_artifact(
            &run_id,
            "workflow-source.json",
            &agent007_workflows::WorkflowSourceRef { workflow_ref },
        ) {
            let summary = format!("workflow setup failed: {}", error);
            let _ = run_store.finish_run(&run_id, false, &summary);
            return Err(anyhow::anyhow!(summary));
        }
    }

    match runner.run(&def, &task).await {
        Ok(result) => {
            let mut report = format!(
                "# Workflow: {}\nTask: {}\nRun ID: {}\nSteps completed: {}/{}\n\n",
                def.name, task, run_id, result.steps_completed, result.steps_total
            );
            for (key, value) in &result.outputs {
                report.push_str(&format!("## {}\n{}\n\n", key, value));
            }
            let _ = stack.run_store.finish_run(&run_id, true, &report);
            Ok(report)
        }
        Err(error) => match &error {
            agent007_workflows::WorkflowError::ApprovalRequired { id } => {
                let pending = stack
                    .run_store
                    .read_json_artifact_optional::<agent007_workflows::WorkflowRunState>(
                        &run_id,
                        "workflow-state.json",
                    )
                    .ok()
                    .flatten()
                    .and_then(|state| state.pending_approval);
                let pending_content = pending
                    .as_ref()
                    .map(|approval| approval.content.as_str())
                    .unwrap_or("");
                let summary = format!(
                    "Workflow '{}' is waiting for approval on step '{}'. Run ID: {}\n\n\
                     Pending approval content:\n{}\n\n\
                     Continue in this same client:\n\
                     1. Review the content above with the user.\n\
                     2. Call agent007_workflow_approve with session={}, step={}, decision=approve|edit|deny.\n\
                     3. If decision=edit, pass the revised content.\n\
                     4. Call agent007_workflow_resume with session={}.\n",
                    def.name,
                    id,
                    run_id,
                    if pending_content.is_empty() {
                        "(content unavailable)"
                    } else {
                        pending_content
                    },
                    run_id,
                    id,
                    run_id,
                );
                let _ = stack.run_store.finish_run_with_status(
                    &run_id,
                    agent007_core::run_store::RunStatus::AwaitingApproval,
                    &summary,
                );
                Ok(summary)
            }
            _ => {
                let _ = stack
                    .run_store
                    .finish_run(&run_id, false, error.to_string());
                Err(anyhow::anyhow!("workflow run failed: {}", error))
            }
        },
    }
}

// ── git helpers ───────────────────────────────────────────────────────────────

fn git_run(args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run git: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "git {} failed (exit {}): {}",
            args.join(" "),
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    Ok(if stdout.is_empty() { stderr } else { stdout })
}

// ── zone check helper ─────────────────────────────────────────────────────────

fn zone_check(config: &Config, path_str: &str, operation: &str) -> Result<String> {
    use agent007_zones::{FileOp, ZoneChecker, ZoneConfig};
    use std::path::Path;

    let zone_config = ZoneConfig {
        forbidden: config.zones.forbidden.clone(),
        readonly: config.zones.readonly.clone(),
        sensitive: config.zones.sensitive.clone(),
        unrestricted: config.zones.unrestricted.clone(),
    };

    let checker =
        ZoneChecker::new(&zone_config).map_err(|e| anyhow::anyhow!("zone config error: {}", e))?;

    let file_op = match operation.to_lowercase().as_str() {
        "read" => FileOp::Read,
        "write" => FileOp::Write,
        "execute" => FileOp::Write, // map execute to write (most restrictive non-delete)
        other => {
            return Err(anyhow::anyhow!(
                "Unknown operation '{}'. Use 'read', 'write', or 'execute'.",
                other
            ))
        }
    };

    let path = Path::new(path_str);
    let zone = checker.zone_for(path);

    match checker.check(path, file_op) {
        Ok(()) => Ok(format!(
            "ALLOWED: {} on '{}' (zone: {})",
            operation,
            path_str,
            zone.as_str()
        )),
        Err(violation) => Ok(format!(
            "DENIED: {} on '{}' (zone: {}): {}",
            operation,
            path_str,
            zone.as_str(),
            violation
        )),
    }
}

// ── task submit helper ────────────────────────────────────────────────────────

async fn task_submit(config: &Config, task: String, persona: Option<String>) -> Result<String> {
    let task_id = uuid_v4();
    let description = match persona {
        Some(ref p) => format!("[persona:{}] {}", p, task),
        None => task.clone(),
    };

    if standalone_mode_available(config) {
        let (stack, run_id) = create_traced_stack(config, "task-submit", &description).await?;
        let core_task = agent007_core::Task::new(&description);
        match stack.orchestrator.run(core_task).await {
            Ok(_) => {
                let output = format!("Task submitted. ID: {}", task_id);
                let _ = stack.run_store.finish_run(&run_id, true, &output);
                Ok(output)
            }
            Err(error) => {
                let _ = stack
                    .run_store
                    .finish_run(&run_id, false, error.to_string());
                Err(error.into())
            }
        }
    } else {
        let run_id = create_delegate_run("task-submit", &description)?;
        let output = format!(
            "Task accepted in hosted MCP mode. ID: {task_id}\n\
             \n\
             IMPORTANT — hosted-MCP limitation: agent007 cannot spawn background subprocesses \
             in this mode. No independent worker was dispatched. The task must be executed \
             inline by the host LLM.\n\
             \n\
             Recommended alternatives for true background/parallel execution:\n\
             1. Use your host environment's native task/agent spawning (e.g. Copilot task tool \
                with agent_type: general-purpose) and have that agent call \
                agent007_workflow_submit_step to report results back into the workflow.\n\
             2. Use a hosted workflow (agent007_workflow_start) which supports parallel steps \
                via the host LLM's background agent capabilities.\n\
             \n\
             Proceeding with inline execution. Persist important results with \
             agent007_memory_write when complete.\n\
             Task: {description}"
        );
        // task-submit semantics: the submission is the terminal event — finish immediately.
        // Do NOT instruct the host to call agent007_record_tokens; this run is already
        // closed. Calling record_tokens would create a second ModelRequest on top of the
        // hosted token fallback written by finish_run, double-counting tokens.
        let _ = load_run_store().finish_run(&run_id, true, &output);
        Ok(output)
    }
}

/// Minimal UUID v4 (random) without pulling in an extra crate.
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Simple pseudo-random based on time + process id
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let raw = t ^ (pid << 32) ^ (t >> 17);
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (raw & 0xffffffff) as u32,
        ((raw >> 32) & 0xffff) as u16,
        ((raw >> 48) & 0x0fff) as u16,
        (((raw >> 60) & 0x3fff) | 0x8000) as u16,
        (raw >> 76) & 0xffffffffffff_u128,
    )
}

// ── skill create helper ───────────────────────────────────────────────────────

fn skill_create(
    name: &str,
    trigger: &str,
    description: &str,
    template: &str,
    model: &str,
) -> Result<String> {
    let skills_dir = agent007_write_home().join("skills");
    std::fs::create_dir_all(&skills_dir)
        .map_err(|e| anyhow::anyhow!("failed to create skills dir: {}", e))?;

    // Sanitise file name: replace spaces with underscores, keep alphanumeric + _-
    let filename = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let path = skills_dir.join(format!("{}.md", filename));
    let trigger_trimmed = trigger.trim();
    let normalized_trigger = if trigger_trimmed.trim_start_matches('/').is_empty() {
        "/custom-skill".to_string()
    } else if trigger_trimmed.starts_with('/') {
        trigger_trimmed.to_string()
    } else {
        format!("/{trigger_trimmed}")
    };

    let content = format!(
        "---\nname: {}\ntrigger: {}\ndescription: {}\nmodel: {}\n---\n{}\n",
        name, normalized_trigger, description, model, template
    );

    std::fs::write(&path, &content)
        .map_err(|e| anyhow::anyhow!("failed to write skill file: {}", e))?;

    let write_home = agent007_write_home();
    let _ = sync_claude_slash_commands_for_home(&write_home);

    Ok(path.display().to_string())
}

// ── workflow create helper ────────────────────────────────────────────────────

/// Save a workflow YAML to the appropriate workflows directory.
/// Validates the YAML parses as a WorkflowDef before writing.
fn workflow_create(name: &str, yaml: &str, overwrite: bool) -> Result<String> {
    // Validate YAML parses as a workflow before touching the filesystem
    let _def: agent007_workflows::types::WorkflowDef =
        serde_yaml::from_str(yaml).map_err(|e| anyhow::anyhow!("invalid workflow YAML: {}", e))?;

    let workflows_dir = agent007_write_home().join("workflows");
    std::fs::create_dir_all(&workflows_dir)
        .map_err(|e| anyhow::anyhow!("failed to create workflows dir: {}", e))?;

    // Sanitise file name: keep alphanumeric, hyphens, underscores
    let filename: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let path = workflows_dir.join(format!("{}.yaml", filename));

    if path.exists() && !overwrite {
        return Err(anyhow::anyhow!(
            "workflow '{}' already exists at {}. Set overwrite=true to replace it.",
            name,
            path.display()
        ));
    }

    std::fs::write(&path, yaml)
        .map_err(|e| anyhow::anyhow!("failed to write workflow file: {}", e))?;

    let write_home = agent007_write_home();
    let _ = sync_claude_slash_commands_for_home(&write_home);

    Ok(path.display().to_string())
}

// ── config show helper ────────────────────────────────────────────────────────

fn config_show(config: &Config) -> String {
    // Serialise to TOML; fall back to debug repr on error
    toml::to_string_pretty(config).unwrap_or_else(|_| format!("{:?}", config))
}

async fn mcp_tools_list(config: &Config) -> Result<String> {
    let (stack, run_id) =
        create_traced_stack(config, "mcp-tools-list", "list downstream MCP tools").await?;
    let tools = match stack.tool_executor.list_mcp_tools().await {
        Ok(tools) => tools,
        Err(error) => {
            let summary = format!("mcp tools listing failed: {}", error);
            let _ = stack.run_store.finish_run(&run_id, false, &summary);
            return Err(anyhow::anyhow!(summary));
        }
    };
    let payload: Vec<serde_json::Value> = tools
        .into_iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect();
    let output = match serde_json::to_string_pretty(&payload) {
        Ok(output) => output,
        Err(error) => {
            let summary = format!("mcp tools response serialization failed: {}", error);
            let _ = stack.run_store.finish_run(&run_id, false, &summary);
            return Err(anyhow::anyhow!(summary));
        }
    };
    let _ = stack.run_store.finish_run(&run_id, true, &output);
    Ok(output)
}

async fn mcp_tool_call(
    config: &Config,
    agent_id: &agent007_core::types::AgentId,
    name: &str,
    args: serde_json::Value,
) -> Result<String> {
    let trace_task = format!("call downstream MCP tool {name}");
    let (stack, run_id) = create_traced_stack(config, "mcp-tool-call", &trace_task).await?;
    if let Some(violation) = evaluate_persona_tool_policy(name) {
        let warning_payload = serde_json::json!({
            "active_persona": violation.persona,
            "requested_tool": name,
            "allowed_tools": violation.allowed_tools,
            "strict_mode": strict_persona_tool_enforcement(),
            "message": violation.message,
        });
        let _ = stack.run_store.write_json_artifact(
            &run_id,
            "persona-policy-warning.json",
            &warning_payload,
        );
        if strict_persona_tool_enforcement() {
            let summary = format!(
                "persona tool policy blocked MCP tool call: {}",
                violation.message
            );
            let _ = stack.run_store.finish_run(&run_id, false, &summary);
            return Err(anyhow::anyhow!(summary));
        }
        tracing::warn!("{}", violation.message);
    }
    match stack
        .tool_executor
        .call_mcp_tool(agent_id, name, args)
        .await
    {
        Ok(result) => {
            let output = match serde_json::to_string_pretty(&result) {
                Ok(output) => output,
                Err(error) => {
                    let summary = format!("mcp tool response serialization failed: {}", error);
                    let _ = stack.run_store.finish_run(&run_id, false, &summary);
                    return Err(anyhow::anyhow!(summary));
                }
            };
            let _ = stack.run_store.finish_run(&run_id, true, &output);
            Ok(output)
        }
        Err(error) => {
            let _ = stack
                .run_store
                .finish_run(&run_id, false, error.to_string());
            Err(error.into())
        }
    }
}

#[derive(Debug)]
struct PersonaToolPolicyViolation {
    persona: String,
    allowed_tools: Vec<String>,
    message: String,
}

fn strict_persona_tool_enforcement() -> bool {
    std::env::var("AGENT007_ENFORCE_PERSONA_TOOLS")
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn evaluate_persona_tool_policy(tool_name: &str) -> Option<PersonaToolPolicyViolation> {
    let Ok(Some(active_persona)) = memory_read("user", "active_persona") else {
        return None;
    };
    let registry = configured_persona_registry();
    let spec = {
        use agent007_core::PersonaProvider;
        registry.get(&active_persona)
    }?;
    if spec.allowed_tools.is_empty() {
        return None;
    }
    if persona_allows_tool(&spec.allowed_tools, tool_name) {
        return None;
    }
    let persona_name = spec.name.clone();
    let allowed_tools = spec.allowed_tools.clone();
    Some(PersonaToolPolicyViolation {
        persona: persona_name.clone(),
        allowed_tools: allowed_tools.clone(),
        message: format!(
            "active persona '{}' does not allow MCP tool '{}'. Allowed: {}",
            persona_name,
            tool_name,
            allowed_tools.join(", ")
        ),
    })
}

fn persona_allows_tool(allowed_tools: &[String], tool_name: &str) -> bool {
    let name = tool_name.to_ascii_lowercase();
    for allowed in allowed_tools {
        let normalized = allowed.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if matches!(
            normalized.as_str(),
            "*" | "all" | "any" | "mcp" | "mcp_tool" | "mcp_tools"
        ) {
            return true;
        }
        if normalized == name {
            return true;
        }
        if let Some(prefix) = normalized.strip_suffix('*') {
            if !prefix.is_empty() && name.starts_with(prefix) {
                return true;
            }
        }
    }
    false
}

fn run_history(limit: usize) -> Result<String> {
    let runs = load_run_store()
        .list_runs(limit)?
        .into_iter()
        .map(|run| {
            serde_json::json!({
                "id": run.id,
                "kind": run.kind,
                "task": run.task,
                "mode": run.mode,
                "provider": run.provider,
                "status": run.status,
                "started_at": run.started_at,
                "finished_at": run.finished_at,
                "output_preview": run.output_preview,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&runs)?)
}

fn run_show(id: &str) -> Result<String> {
    let store = load_run_store();
    let detail = store.load_run(id)?;
    let workflow_request =
        store.read_json_artifact_optional::<serde_json::Value>(id, "workflow-request.json")?;
    let workflow_source =
        store.read_json_artifact_optional::<serde_json::Value>(id, "workflow-source.json")?;
    let workflow_state =
        store.read_json_artifact_optional::<serde_json::Value>(id, "workflow-state.json")?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "run": detail,
        "workflow_request": workflow_request,
        "workflow_source": workflow_source,
        "workflow_state": workflow_state,
    }))?)
}

fn compact_output_tool(
    config: &Config,
    command: &str,
    output: &str,
    level: &str,
) -> Result<String> {
    let level = parse_compact_level(level)?;
    let run_id = create_recorded_utility_run(config, "compact-output", command)?;
    let store = load_run_store();
    let result: Result<String> = (|| {
        let compact = agent007_core::compact_command_output(command, output, level);
        store.write_text_artifact(&run_id, "raw-output.txt", output)?;
        store.write_text_artifact(&run_id, "compact-output.txt", &compact.summary)?;
        store.write_json_artifact(&run_id, "compact-output.json", &compact)?;
        let _ = store.finish_run(
            &run_id,
            true,
            format!(
                "{} saved ~{} tokens via {} compaction",
                command, compact.tokens_saved, compact.strategy
            ),
        );
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "run_id": run_id,
            "result": compact,
        }))?)
    })();
    if let Err(error) = result {
        let summary = format!("compact-output failed: {}", error);
        let _ = store.finish_run(&run_id, false, &summary);
        return Err(anyhow::anyhow!(summary));
    }
    result
}

fn context_compile_tool(
    config: &Config,
    task: &str,
    max_files: usize,
    max_memory_notes: usize,
    max_prompt_tokens: u64,
    reserve_tokens: u64,
    max_response_tokens: u64,
) -> Result<String> {
    let run_id = create_recorded_utility_run(config, "context-compile", task)?;
    let store = load_run_store();
    let result: Result<String> = (|| {
        let cwd = std::env::current_dir()?;
        let compiler = agent007_core::ContextCompiler::new(
            &cwd,
            agent007_home(),
            agent007_core::TokenBudget {
                max_prompt_tokens,
                reserve_tokens,
                max_response_tokens,
            },
        );
        let bundle = compiler.compile(task, max_files, max_memory_notes)?;
        store.write_json_artifact(&run_id, "context-bundle.json", &bundle)?;
        store.write_text_artifact(&run_id, "compiled-context.txt", &bundle.compiled_context)?;
        let _ = store.finish_run(
            &run_id,
            true,
            format!(
                "compiled context at {} level (~{} tokens)",
                bundle.recommended_level.as_str(),
                bundle.estimated_tokens
            ),
        );
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "run_id": run_id,
            "bundle": bundle,
        }))?)
    })();
    if let Err(error) = result {
        let summary = format!("context-compile failed: {}", error);
        let _ = store.finish_run(&run_id, false, &summary);
        return Err(anyhow::anyhow!(summary));
    }
    result
}

fn repo_brain_markdown(brain: &agent007_core::RepoBrain) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Repo Brain: {}\n\n", brain.project_name));
    out.push_str(&format!("{}\n\n", brain.summary));
    if !brain.ecosystems.is_empty() {
        out.push_str("## Ecosystems\n");
        for item in &brain.ecosystems {
            out.push_str(&format!("- {}\n", item));
        }
        out.push('\n');
    }
    if !brain.entrypoints.is_empty() {
        out.push_str("## Entrypoints\n");
        for item in &brain.entrypoints {
            out.push_str(&format!("- {}\n", item));
        }
        out.push('\n');
    }
    if !brain.recommended_commands.is_empty() {
        out.push_str("## Recommended Commands\n");
        for item in &brain.recommended_commands {
            out.push_str(&format!("- {}\n", item));
        }
        out.push('\n');
    }
    if !brain.conventions.is_empty() {
        out.push_str("## Conventions\n");
        for item in &brain.conventions {
            out.push_str(&format!("- {}\n", item));
        }
        out.push('\n');
    }
    if !brain.workflows.is_empty() {
        out.push_str("## Workflows\n");
        for item in &brain.workflows {
            out.push_str(&format!("- {}\n", item));
        }
        out.push('\n');
    }
    if !brain.skills.is_empty() {
        out.push_str("## Skills\n");
        for item in &brain.skills {
            out.push_str(&format!("- {}\n", item));
        }
        out.push('\n');
    }
    if !brain.memory_notes.is_empty() {
        out.push_str("## Memory Notes\n");
        for item in &brain.memory_notes {
            out.push_str(&format!("- {}\n", item));
        }
    }
    out
}

fn repo_brain_refresh_tool(config: &Config) -> Result<String> {
    let run_id = create_recorded_utility_run(config, "repo-brain-refresh", "distill repo brain")?;
    let store = load_run_store();
    let result: Result<String> = (|| {
        let cwd = std::env::current_dir()?;
        let brain = agent007_core::RepoBrainBuilder::new(&cwd, agent007_home()).build()?;
        let markdown = repo_brain_markdown(&brain);
        memory_write("project", "repo_brain", &markdown)?;
        store.write_json_artifact(&run_id, "repo-brain.json", &brain)?;
        store.write_text_artifact(&run_id, "repo-brain.md", &markdown)?;
        let _ = store.finish_run(
            &run_id,
            true,
            format!("refreshed repo brain for {}", brain.project_name),
        );
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "run_id": run_id,
            "memory_key": "project/repo_brain",
            "repo_brain": brain,
            "markdown": markdown,
        }))?)
    })();
    if let Err(error) = result {
        let summary = format!("repo-brain-refresh failed: {}", error);
        let _ = store.finish_run(&run_id, false, &summary);
        return Err(anyhow::anyhow!(summary));
    }
    result
}

fn budget_estimate_tool(
    config: &Config,
    task: Option<String>,
    text: Option<String>,
    max_prompt_tokens: u64,
    reserve_tokens: u64,
    max_response_tokens: u64,
) -> Result<String> {
    let text = text.or_else(|| task.clone()).unwrap_or_default();
    if text.trim().is_empty() {
        return Err(anyhow::anyhow!("provide at least one of: task or text"));
    }

    let budget = agent007_core::TokenBudget {
        max_prompt_tokens,
        reserve_tokens,
        max_response_tokens,
    };
    let report = budget.estimate_prompt(agent007_core::estimate_tokens(&text));
    let run_id = create_recorded_utility_run(
        config,
        "budget-estimate",
        task.as_deref().unwrap_or("budget estimate"),
    )?;
    let store = load_run_store();
    let result: Result<String> = (|| {
        store.write_text_artifact(&run_id, "budget-input.txt", &text)?;
        store.write_json_artifact(&run_id, "budget-report.json", &report)?;
        let _ = store.finish_run(
            &run_id,
            true,
            format!(
                "budget recommends {} context (remaining prompt tokens: {})",
                report.recommended_level.as_str(),
                report.remaining_prompt_tokens
            ),
        );
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "run_id": run_id,
            "task": task,
            "report": report,
        }))?)
    })();
    if let Err(error) = result {
        let summary = format!("budget-estimate failed: {}", error);
        let _ = store.finish_run(&run_id, false, &summary);
        return Err(anyhow::anyhow!(summary));
    }
    result
}

// ── health check helper ───────────────────────────────────────────────────────

fn health_check(config: &Config) -> String {
    let home = agent007_write_home();

    let memory_dir = home.join("memory");

    let memory_ok = memory_dir.exists();

    let skills_count = load_available_skills()
        .map(|skills| skills.len())
        .unwrap_or(0);
    let personas_count = {
        // Built-in count + user overrides
        let registry = configured_persona_registry();
        use agent007_core::PersonaProvider;
        registry.list().len()
    };

    let zones_configured = !config.zones.forbidden.is_empty()
        || !config.zones.readonly.is_empty()
        || !config.zones.sensitive.is_empty()
        || !config.zones.unrestricted.is_empty();

    let available_providers = {
        let mut providers = Vec::new();
        if std::env::var("ANTHROPIC_API_KEY")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            providers.push("claude");
        }
        if std::env::var("OPENAI_API_KEY")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            providers.push("codex");
        }
        if selected_runtime_provider(config).as_deref() == Some("ollama") {
            providers.push("ollama");
        }
        if std::env::var("AGENT007_DRY_RUN").is_ok() {
            providers.push("mock");
        }
        if providers.is_empty() {
            "none".to_string()
        } else {
            providers.join(", ")
        }
    };
    let selected_provider =
        selected_runtime_provider(config).unwrap_or_else(|| "hosted-mcp".to_string());
    let selected_model = selected_runtime_model(config).unwrap_or_else(|| "host-llm".to_string());
    let runtime_mode = runtime_mode_label(config);
    let (skill_collisions, workflow_collisions) = catalog_collision_counts();

    format!(
        "agent007 health\n\
         ───────────────\n\
         home:              {}\n\
         memory dir:        {} ({})\n\
         skills:            {} loaded\n\
         personas:          {} available\n\
         zones configured:  {}\n\
         runtime mode:      {}\n\
         providers:         {}\n\
         selected provider: {}\n\
         selected model:    {}\n\
         collisions:        skills={} workflows={}\n",
        home.display(),
        memory_dir.display(),
        if memory_ok { "exists" } else { "missing" },
        skills_count,
        personas_count,
        if zones_configured { "yes" } else { "no" },
        runtime_mode,
        available_providers,
        selected_provider,
        selected_model,
        skill_collisions,
        workflow_collisions,
    )
}

// ── workflow plan (hosted MCP mode) ────────────────────────────────────────

async fn workflow_plan(_config: &Config, name: &str, task: &str) -> Result<String> {
    let def = load_workflow_def(name)?;
    let registry = configured_persona_registry();

    // Load ranked memory context for template variable injection.
    let mem_ctx = build_memory_context(task);

    let mut steps = Vec::new();
    for step in &def.steps {
        let persona = {
            use agent007_core::PersonaProvider;
            registry.get(&step.agent)
        };

        let rendered_prompt = mem_ctx.apply_to(
            &step
                .prompt
                .as_deref()
                .unwrap_or("")
                .replace("{{task}}", task),
        );

        let mut step_json = serde_json::json!({
            "id": step.id,
            "agent": step.agent,
            "prompt": rendered_prompt,
            "depends_on": step.depends_on.clone().unwrap_or_default(),
        });

        if let Some(output) = &step.output {
            step_json["output_var"] = serde_json::json!(output);
        }

        if let Some(spec) = persona {
            step_json["persona"] = serde_json::json!({
                "system_prompt": spec.system_prompt,
                "preferred_model": spec.preferred_model,
                "allowed_tools": spec.allowed_tools,
            });
        }

        steps.push(step_json);
    }

    let plan = serde_json::json!({
        "workflow": def.name,
        "description": def.description,
        "mode": "hosted-mcp",
        "instructions": "Execute each step in dependency order. Steps with no dependencies \
                         can run in parallel. For each step, use the persona's system_prompt \
                         as context, execute the rendered prompt, and store the output. \
                         Substitute {{output_var}} references in later steps with earlier results. \
                         When all steps complete, synthesize a final report.",
        "steps": steps,
    });

    Ok(serde_json::to_string_pretty(&plan)?)
}

// ── agent wizard helpers ──────────────────────────────────────────────────

fn agent_catalog() -> String {
    let registry = configured_persona_registry();

    use agent007_core::PersonaProvider;
    let personas = registry.list();

    let archetypes: Vec<serde_json::Value> = personas
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "description": p.description,
                "preferred_model": p.preferred_model,
                "allowed_tools": p.allowed_tools,
                "skills": p.skills,
                "agent_type": p.agent_type,
                "allowed_workers": p.allowed_workers,
                "memory_namespace": p.memory_namespace,
                "zones": p.zones,
                "system_prompt": p.system_prompt,
            })
        })
        .collect();

    let catalog = serde_json::json!({
        "archetypes": archetypes,
        "total": archetypes.len(),
        "instructions": "Pick an archetype and customize it for your specific needs, or create \
                         a new agent from scratch. Call agent007_agent_create with action='save' \
                         and provide: name, description, system_prompt, preferred_model, and \
                         allowed_tools. The host LLM should generate a detailed system_prompt \
                         tailored to the user's specific domain and requirements.",
    });

    serde_json::to_string_pretty(&catalog).unwrap_or_default()
}

fn agent_save(
    name: &str,
    description: &str,
    system_prompt: &str,
    preferred_model: &str,
    allowed_tools: Option<&[String]>,
    skills: Option<&[String]>,
    agent_type: Option<&str>,
    allowed_workers: Option<&[String]>,
    memory_namespace: Option<&str>,
) -> Result<String> {
    let personas_dir = agent007_write_home().join("personas");
    std::fs::create_dir_all(&personas_dir)
        .map_err(|e| anyhow::anyhow!("failed to create personas dir: {}", e))?;

    let filename = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase();
    let path = personas_dir.join(format!("{filename}.toml"));

    let existing = read_existing_persona_spec(&path);
    let allowed_tools = allowed_tools
        .map(|values| values.to_vec())
        .or_else(|| existing.as_ref().map(|spec| spec.allowed_tools.clone()))
        .unwrap_or_default();
    let skills = skills
        .map(|values| values.to_vec())
        .or_else(|| existing.as_ref().map(|spec| spec.skills.clone()))
        .unwrap_or_default();
    let allowed_workers = allowed_workers
        .map(|values| values.to_vec())
        .or_else(|| {
            existing
                .as_ref()
                .and_then(|spec| spec.allowed_workers.clone())
        })
        .unwrap_or_default();
    let agent_type = agent_type
        .map(str::to_string)
        .or_else(|| existing.as_ref().and_then(|spec| spec.agent_type.clone()));
    let memory_namespace = memory_namespace.map(str::to_string).or_else(|| {
        existing
            .as_ref()
            .and_then(|spec| spec.memory_namespace.clone())
    });
    let zones = existing.as_ref().and_then(|spec| spec.zones.clone());

    let mut content = format!(
        "name            = \"{}\"\n\
         description     = \"{}\"\n\
         preferred_model = \"{}\"\n\
         allowed_tools   = [{}]\n\
         skills          = [{}]\n",
        name,
        description.replace('"', "\\\""),
        preferred_model,
        toml_string_array(&allowed_tools),
        toml_string_array(&skills),
    );
    if let Some(agent_type) = agent_type.filter(|value| !value.trim().is_empty()) {
        content.push_str(&format!(
            "agent_type      = \"{}\"\n",
            agent_type.replace('"', "\\\"")
        ));
    }
    if !allowed_workers.is_empty() {
        content.push_str(&format!(
            "allowed_workers = [{}]\n",
            toml_string_array(&allowed_workers)
        ));
    }
    if let Some(memory_namespace) = memory_namespace.filter(|value| !value.trim().is_empty()) {
        content.push_str(&format!(
            "memory_namespace = \"{}\"\n",
            memory_namespace.replace('"', "\\\"")
        ));
    }
    if let Some(zones) = zones {
        content.push_str("\n[zones]\n");
        if !zones.forbidden.is_empty() {
            content.push_str(&format!(
                "forbidden = [{}]\n",
                toml_string_array(&zones.forbidden)
            ));
        }
        if !zones.readonly.is_empty() {
            content.push_str(&format!(
                "readonly = [{}]\n",
                toml_string_array(&zones.readonly)
            ));
        }
        if !zones.sensitive.is_empty() {
            content.push_str(&format!(
                "sensitive = [{}]\n",
                toml_string_array(&zones.sensitive)
            ));
        }
    }
    content.push_str(&format!(
        "\nsystem_prompt   = \"\"\"\n{}\n\"\"\"\n",
        system_prompt
    ));

    std::fs::write(&path, &content)
        .map_err(|e| anyhow::anyhow!("failed to write persona file: {}", e))?;

    Ok(path.display().to_string())
}

fn toml_string_array(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("\"{}\"", value.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ")
}

fn read_existing_persona_spec(path: &std::path::Path) -> Option<PersonaSpec> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

// ── skill wizard helpers ──────────────────────────────────────────────────

fn skill_templates() -> String {
    let templates = serde_json::json!({
        "templates": [
            {
                "id": "code-review",
                "name": "Code Review",
                "description": "Review code for quality, security, performance, and best practices",
                "trigger": "/review",
                "template": "Review the following code for quality, security vulnerabilities, \
                             performance issues, and adherence to best practices.\n\n\
                             Code:\n{{args}}\n\n\
                             Provide findings organized by severity (Critical/High/Medium/Low) \
                             with specific line references and actionable fixes.",
                "model": "codex"
            },
            {
                "id": "explain",
                "name": "Code Explanation",
                "description": "Explain code in detail with architecture and data flow",
                "trigger": "/explain",
                "template": "Explain the following code in detail. Cover:\n\
                             1. Purpose and high-level architecture\n\
                             2. Data flow and key abstractions\n\
                             3. Important design decisions and trade-offs\n\
                             4. Potential improvements\n\n\
                             Code:\n{{args}}",
                "model": "codex"
            },
            {
                "id": "test-gen",
                "name": "Test Generator",
                "description": "Generate comprehensive tests for code",
                "trigger": "/test-gen",
                "template": "Generate comprehensive tests for the following code. Include:\n\
                             - Happy path tests\n\
                             - Error/edge case tests\n\
                             - Boundary condition tests\n\
                             - Use the project's testing framework and conventions.\n\n\
                             Code:\n{{args}}",
                "model": "codex"
            },
            {
                "id": "refactor",
                "name": "Refactoring Advisor",
                "description": "Suggest refactoring improvements with before/after examples",
                "trigger": "/refactor",
                "template": "Analyze the following code and suggest refactoring improvements.\n\
                             For each suggestion provide:\n\
                             - What to change and why\n\
                             - Before/after code examples\n\
                             - Impact on maintainability, readability, and performance\n\n\
                             Code:\n{{args}}",
                "model": "codex"
            },
            {
                "id": "blank",
                "name": "Custom Skill",
                "description": "Create a skill from scratch with your own prompt template",
                "trigger": "/my-skill",
                "template": "{{args}}",
                "model": "codex"
            }
        ],
        "instructions": "Pick a template, customize it for your needs, then call \
                         agent007_skill_wizard with action='save' and provide: name, trigger, \
                         description, template (prompt body with {{args}} placeholder), and \
                         optionally model."
    });

    serde_json::to_string_pretty(&templates).unwrap_or_default()
}

/// Entry point: start MCP stdio server + web dashboard (unless `--no-dashboard`).
pub async fn execute(config: Arc<Config>, dashboard_port: u16, no_dashboard: bool) -> Result<()> {
    // On every server start, close out any runs left open by a previous crash.
    let stale = load_run_store().cleanup_stale_runs();
    if stale > 0 {
        eprintln!("[agent007] cleaned up {stale} stale run(s) from previous session");
    }

    let mut shared_dispatcher: Option<Arc<LocalDispatcher>> = None;
    let mut shared_learning: Option<Arc<LearningDispatcher>> = None;
    // Keep runtime stack alive for the full server lifetime so background
    // workers (feedback collector, optimizer loop, etc.) are not dropped.
    let mut _runtime_stack: Option<super::run::Stack> = None;

    if !no_dashboard {
        // Guard: if another process already owns the dashboard port AND it's serving the
        // same project, skip starting a new instance. If a different project owns the port
        // (e.g. Copilot or Cursor opened another project first), start a new dashboard on
        // a different port so each project gets its own.
        let already_running = if let Some(port) = read_dashboard_port() {
            if dashboard_port_is_live(port).await && dashboard_port_is_same_project(port).await {
                eprintln!("[agent007] web dashboard already running: http://localhost:{port} — skipping new instance");
                true
            } else {
                false
            }
        } else {
            false
        };

        if already_running {
            // Still wire up a dispatcher so MCP tool calls publish events — the live
            // dashboard belongs to the first process but run history is shared on disk.
            let stack = build_stack_for_web(&config).await?;
            shared_dispatcher = Some(stack.dispatcher.clone());
            shared_learning = Some(stack.learning_dispatcher.clone());
            let collector = stack.feedback_collector.clone();
            stack.tracker.spawn(async move {
                if let Err(e) = collector.run().await {
                    tracing::warn!("feedback collector error: {e}");
                }
            });
            super::run::spawn_learning_runtime_workers(&stack);
            _runtime_stack = Some(stack);
        } else {
            // Start the dashboard inline regardless of whether stdin is a terminal.
            // The MCP stdio protocol and the HTTP web server use completely different
            // transports and coexist in the same process without conflict.
            let stack = build_stack_for_web(&config).await?;
            let standalone_mode = standalone_mode_available(&config);
            let runtime_mode = runtime_mode_label(&config).to_string();
            let provider_label = match (
                selected_runtime_provider(&config),
                selected_runtime_model(&config),
            ) {
                (Some(provider), Some(model)) if provider != model => {
                    format!("{provider} / {model}")
                }
                (Some(provider), _) => provider,
                _ => "hosted-mcp".to_string(),
            };

            shared_dispatcher = Some(stack.dispatcher.clone());
            shared_learning = Some(stack.learning_dispatcher.clone());

            let collector = stack.feedback_collector.clone();
            stack.tracker.spawn(async move {
                if let Err(e) = collector.run().await {
                    tracing::warn!("feedback collector error: {e}");
                }
            });
            super::run::spawn_learning_runtime_workers(&stack);

            let provider_readiness = provider_readiness_response(&config);
            let web = agent007_web::WebServer::new_with_provider_readiness(
                stack.dispatcher.clone(),
                stack.learning_dispatcher.clone(),
                stack.model_router.clone(),
                Some(stack.workflow_runner.clone()),
                stack.cancel.clone(),
                standalone_mode,
                runtime_mode,
                provider_label,
                provider_readiness,
            );

            tokio::spawn(async move {
                match find_free_port_with_listener(dashboard_port).await {
                    Ok((actual_port, listener)) => {
                        eprintln!("[agent007] web dashboard: http://localhost:{actual_port}");
                        persist_dashboard_port(actual_port);
                        if let Err(e) = web.run_with_listener(listener).await {
                            eprintln!("[agent007] web dashboard error: {e}");
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "[agent007] web dashboard disabled: could not bind a local port ({e})"
                        );
                    }
                }
            });

            tracing::info!(
                "agent007 MCP server starting (stdio) + web dashboard on port {dashboard_port}"
            );
            _runtime_stack = Some(stack);
        } // end else already_running
    } else {
        if let Err(error) = ensure_dashboard_sidecar(dashboard_port).await {
            tracing::warn!("failed to ensure dashboard sidecar: {error}");
        }
        tracing::info!("agent007 MCP server starting (stdio transport, dashboard disabled)");
    }

    let mut server = Agent007Server::new(config);
    if let (Some(d), Some(l)) = (shared_dispatcher, shared_learning) {
        server = server.with_dispatchers(d, l);
    }
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

/// Try `preferred`, then increment until a bindable port is found.
/// Find a free port and return both the port number and the already-bound listener,
/// so the caller can pass the listener directly to the HTTP server (avoids TOCTOU race
/// where two processes check the same port, both see it free, and one fails to bind).
async fn find_free_port_with_listener(
    preferred: u16,
) -> std::io::Result<(u16, tokio::net::TcpListener)> {
    let mut last_err: Option<std::io::Error> = None;
    for offset in 0u16..50 {
        let port = preferred.wrapping_add(offset);
        let addr = format!("127.0.0.1:{port}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => return Ok((port, listener)),
            Err(e) => last_err = Some(e),
        }
    }
    // Last-ditch: let the OS assign any free localhost port.
    match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => {
            let port = listener.local_addr().map(|a| a.port()).unwrap_or(preferred);
            Ok((port, listener))
        }
        Err(fallback_err) => {
            if let Some(first_err) = last_err {
                Err(std::io::Error::new(
                    fallback_err.kind(),
                    format!(
                        "preferred dashboard ports failed ({first_err}); fallback bind failed ({fallback_err})"
                    ),
                ))
            } else {
                Err(fallback_err)
            }
        }
    }
}

/// Write the active dashboard port to `.agent007/memory/project/dashboard_port.md`
/// so other tools (TUI, scripts, health checks) can discover it.
/// Project-local — each project has its own serve instance on its own port.
fn persist_dashboard_port(port: u16) {
    let store = memory_store();
    let scoped = store.scoped("project");
    let _ = scoped.write("dashboard_port", &port.to_string());
    let _ = scoped.write("dashboard_url", &format!("http://localhost:{port}"));
    register_dashboard_port_for_current_project(port);
}

#[allow(dead_code)]
async fn ensure_dashboard_sidecar(preferred_port: u16) -> Result<Option<u16>> {
    if let Some(port) = read_dashboard_port() {
        if dashboard_port_is_live(port).await {
            return Ok(Some(port));
        }
    }
    if let Some(port) = read_registered_dashboard_port_for_current_project() {
        if dashboard_port_is_live(port).await {
            persist_dashboard_port(port);
            return Ok(Some(port));
        }
    }

    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("serve-web")
        .arg("--port")
        .arg(preferred_port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(cwd) = std::env::current_dir() {
        cmd.current_dir(cwd);
    }
    cmd.spawn()?;

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if let Some(port) = read_dashboard_port() {
            if dashboard_port_is_live(port).await {
                return Ok(Some(port));
            }
        }
    }

    Ok(read_dashboard_port())
}

fn read_dashboard_port() -> Option<u16> {
    let store = memory_store();
    let raw = store.scoped("project").read("dashboard_port").ok()??;
    raw.trim().parse().ok()
}

fn current_project_registry_key() -> Option<String> {
    let write_home = agent007_write_home();
    let project_root = write_home.parent()?;
    let canonical = project_root.canonicalize().ok();
    Some(
        canonical
            .unwrap_or_else(|| project_root.to_path_buf())
            .to_string_lossy()
            .to_string(),
    )
}

fn read_registered_dashboard_port_for_current_project() -> Option<u16> {
    let key = current_project_registry_key()?;
    let path = agent007_global_home().join("ports.toml");
    let raw = std::fs::read_to_string(path).ok()?;
    let value = raw.parse::<toml::Value>().ok()?;
    value
        .get("projects")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get(&key))
        .and_then(|v| v.as_integer())
        .and_then(|v| u16::try_from(v).ok())
}

fn register_dashboard_port_for_current_project(port: u16) {
    let Some(key) = current_project_registry_key() else {
        return;
    };
    let path = agent007_global_home().join("ports.toml");
    let mut root = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| raw.parse::<toml::Value>().ok())
        .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()));

    let Some(root_table) = root.as_table_mut() else {
        return;
    };
    let projects_entry = root_table
        .entry("projects".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let Some(projects_table) = projects_entry.as_table_mut() else {
        return;
    };
    projects_table.insert(key, toml::Value::Integer(i64::from(port)));

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(raw) = toml::to_string_pretty(&root) {
        let _ = std::fs::write(path, raw);
    }
}

async fn dashboard_port_is_live(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tokio::time::timeout(
        Duration::from_millis(200),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

/// Check if the dashboard on `port` is serving the same project as the current process.
/// Issues a raw HTTP GET /api/stats and looks for the project_path in the JSON body.
/// Returns false on network/parse errors so callers can safely start a new dashboard
/// instead of silently skipping startup.
async fn dashboard_port_is_same_project(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let current_home = agent007_write_home();
    let current_project = current_home
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let connect = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"));
    let Ok(Ok(mut stream)) = tokio::time::timeout(Duration::from_millis(400), connect).await else {
        return false;
    };

    let req = b"GET /api/stats HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(req).await.is_err() {
        return false;
    }

    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_millis(400), stream.read_to_end(&mut buf));
    if read.await.is_err() {
        return false;
    }

    let body = String::from_utf8_lossy(&buf);
    // JSON body starts after the blank line separating headers from body
    let json_str = body.split("\r\n\r\n").nth(1).unwrap_or("");
    let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return false;
    };

    let running_project = json
        .get("project_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    running_project == current_project
}

// ── ETR (Embedded Tool Runtime) helpers ──────────────────────────────────────

fn etr_call(tool_name: &str, input: serde_json::Value, compact: bool) -> String {
    use agent007_etr::{EtrCallRequest, EtrDispatcher};
    let workspace_root = agent007_home();
    let dispatcher = EtrDispatcher::new(workspace_root);
    let req = EtrCallRequest {
        tool: tool_name.to_string(),
        input,
        compact,
    };
    let result = dispatcher.call(req);
    serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"))
}

fn etr_list() -> String {
    use agent007_etr::EtrDispatcher;
    let workspace_root = agent007_home();
    let dispatcher = EtrDispatcher::new(workspace_root);
    let tools = dispatcher.list_tools();
    serde_json::to_string_pretty(&serde_json::json!({ "tools": tools }))
        .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;
    use std::thread;

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn unset(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, original }
        }

        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn write_workflow_fixture(dir: &std::path::Path, name: &str, body: &str) {
        let workflows_dir = dir.join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(workflows_dir.join(format!("{name}.toml")), body).unwrap();
    }

    fn write_skill_fixture(
        dir: &std::path::Path,
        file_name: &str,
        trigger: &str,
        name: &str,
        description: &str,
    ) {
        let skills_dir = dir.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join(format!("{file_name}.md")),
            format!(
                "---\nname: {name}\ndescription: {description}\ntrigger: {trigger}\nmodel: codex\n---\nDo work for {{{{args}}}}.\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn tool_defs_contains_persona_list_and_show() {
        let defs = Agent007Server::tool_defs();
        let names: Vec<&str> = defs.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            names.contains(&"agent007_persona_list"),
            "missing agent007_persona_list"
        );
        assert!(
            names.contains(&"agent007_persona_show"),
            "missing agent007_persona_show"
        );
    }

    #[test]
    fn tool_defs_has_at_least_30_tools() {
        let defs = Agent007Server::tool_defs();
        assert!(
            defs.len() >= 30,
            "expected at least 30 tools, got {}",
            defs.len()
        );
    }

    #[test]
    fn tool_defs_contains_all_new_tools() {
        let defs = Agent007Server::tool_defs();
        let names: Vec<&str> = defs.iter().map(|t| t.name.as_ref()).collect();
        let expected = [
            "agent007_dispatch",
            "agent007_memory_read",
            "agent007_memory_write",
            "agent007_memory_list",
            "agent007_record_tokens",
            "agent007_workflow_list",
            "agent007_workflow_run",
            "agent007_workflow_resume",
            "agent007_workflow_approve",
            "agent007_workflow_start",
            "agent007_workflow_next",
            "agent007_workflow_submit_step",
            "agent007_workflow_status",
            "agent007_workflow_get_output",
            "agent007_workflow_heartbeat",
            "agent007_git_status",
            "agent007_git_diff",
            "agent007_git_log",
            "agent007_git_commit",
            "agent007_persona_switch",
            "agent007_zone_check",
            "agent007_task_submit",
            "agent007_skill_create",
            "agent007_config_show",
            "agent007_health",
            "agent007_mcp_tools_list",
            "agent007_mcp_tool_call",
            "agent007_run_history",
            "agent007_run_show",
            "agent007_compact_output",
            "agent007_context_compile",
            "agent007_repo_brain_refresh",
            "agent007_budget_estimate",
        ];
        for tool_name in &expected {
            assert!(names.contains(tool_name), "missing tool: {}", tool_name);
        }
    }

    #[test]
    fn tool_defs_include_dynamic_skill_tools() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        write_skill_fixture(
            tmp.path(),
            "review-pr",
            "/agent007/review-pr",
            "Review PR",
            "Review a pull request",
        );

        let defs = Agent007Server::tool_defs();
        let names: Vec<&str> = defs.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"agent007_skill_review_pr"));
        assert!(!names.contains(&"agent007_skill_agent007_review_pr"));

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn tool_defs_include_dynamic_workflow_tools() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        write_workflow_fixture(
            tmp.path(),
            "code-review",
            r#"
name = "Code Review"
description = "Review a code change"

[[steps]]
id = "review"
agent = "Reviewer"
prompt = "Review {{task}}"
output = "notes"
"#,
        );

        let defs = Agent007Server::tool_defs();
        let names: Vec<&str> = defs.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"agent007_workflow_code_review"));

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn dynamic_skill_tool_accepts_legacy_aliases() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        write_skill_fixture(
            tmp.path(),
            "write-tests",
            "/agent007/write-tests",
            "Write Tests",
            "Write tests for code",
        );

        let primary = dynamic_skill_tool("agent007_skill_write_tests");
        assert!(primary.is_some());
        let legacy = dynamic_skill_tool("agent007_skill_agent007_write_tests");
        assert!(legacy.is_some());

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn help_lists_dynamic_skill_and_workflow_names() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        write_skill_fixture(
            tmp.path(),
            "write-tests",
            "/agent007/write-tests",
            "Write Tests",
            "Write tests for code",
        );
        write_workflow_fixture(
            tmp.path(),
            "tdd",
            r#"
name = "TDD"
description = "Red green refactor"

[[steps]]
id = "red"
agent = "Tester"
prompt = "Write a failing test for {{task}}"
output = "red"
"#,
        );

        let help = agent007_help(None);
        assert!(help.contains("agent007_skill_write_tests"));
        assert!(help.contains("agent007_workflow_tdd"));
        assert!(help.contains("How to invoke from Codex"));

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn parse_dispatch_command_supports_workflow_alias_syntax() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        write_workflow_fixture(
            tmp.path(),
            "code-review",
            r#"
name = "Code Review"

[[steps]]
id = "review"
agent = "Reviewer"
prompt = "Review {{task}}"
output = "notes"
"#,
        );

        let parsed =
            parse_dispatch_command("$agent007 wf code_review review current diff").unwrap();
        assert_eq!(
            parsed,
            DispatchCommand::WorkflowRun {
                name: "code-review".to_string(),
                task: "review current diff".to_string(),
            }
        );

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn parse_dispatch_command_supports_skill_shorthand() {
        let parsed = parse_dispatch_command("$agent007 skill brainstorm onboarding ideas").unwrap();
        assert_eq!(
            parsed,
            DispatchCommand::SkillRun {
                trigger: "/brainstorm".to_string(),
                args: "onboarding ideas".to_string(),
            }
        );
    }

    #[test]
    fn parse_dispatch_command_defaults_to_run_for_plain_text() {
        let parsed = parse_dispatch_command("$agent007 refactor auth module").unwrap();
        assert_eq!(
            parsed,
            DispatchCommand::Run {
                task: "refactor auth module".to_string(),
            }
        );
    }

    #[test]
    fn parse_dispatch_command_returns_help_for_empty_input() {
        let parsed = parse_dispatch_command("   ").unwrap();
        assert_eq!(
            parsed,
            DispatchCommand::Help {
                topic: Some("overview".to_string()),
            }
        );
    }

    #[test]
    fn record_actual_tokens_recovers_stale_restart_failure() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());

        let run_id = create_delegate_run("workflow", "recover me").unwrap();
        assert_eq!(load_run_store().cleanup_stale_runs(), 1);

        let message =
            record_actual_tokens(&run_id, 2800, "host-llm", Some("final output")).unwrap();
        assert!(message.contains("Recovered stale run"));

        let detail = load_run_store().load_run(&run_id).unwrap();
        assert!(matches!(
            detail.metadata.status,
            agent007_core::run_store::RunStatus::Succeeded
        ));
        assert_eq!(detail.metadata.provider.as_deref(), Some("host-llm"));
        assert_eq!(
            detail.metadata.output_preview.as_deref(),
            Some("final output")
        );

        let summary: agent007_core::run_store::RunTokenSummary = load_run_store()
            .read_json_artifact(&run_id, "token-summary.json")
            .unwrap();
        assert_eq!(summary.tokens, 2800);
        assert_eq!(summary.requests, 1);

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn record_actual_tokens_updates_completed_delegate_run() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());

        let run_id = create_delegate_run("skill", "finish hosted skill").unwrap();
        mark_delegate_run_handed_off(&run_id, "delegated to host LLM", "handoff").unwrap();

        let handed_off = load_run_store().load_run(&run_id).unwrap();
        assert!(matches!(
            handed_off.metadata.status,
            agent007_core::run_store::RunStatus::Succeeded
        ));
        assert_eq!(
            handed_off.metadata.output_preview.as_deref(),
            Some("delegated to host LLM")
        );

        let message =
            record_actual_tokens(&run_id, 512, "host-llm", Some("final hosted output")).unwrap();
        assert!(message.contains("Recorded 512 tokens"));

        let detail = load_run_store().load_run(&run_id).unwrap();
        assert!(matches!(
            detail.metadata.status,
            agent007_core::run_store::RunStatus::Succeeded
        ));
        assert_eq!(detail.metadata.provider.as_deref(), Some("host-llm"));
        assert_eq!(
            detail.metadata.output_preview.as_deref(),
            Some("final hosted output")
        );

        let stored_output = load_run_store()
            .read_text_artifact_optional(&run_id, "output.txt")
            .unwrap()
            .unwrap();
        assert_eq!(stored_output, "final hosted output");

        let delegate_state = hosted_delegate_state(&load_run_store(), &run_id).unwrap();
        assert!(!delegate_state.awaiting_host_report);

        let key = format!("skill_{}", &run_id[..8.min(run_id.len())]);
        let memory = memory_store()
            .scoped("project")
            .read(&key)
            .unwrap()
            .unwrap();
        assert_eq!(memory, "final hosted output");

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn record_actual_tokens_uses_kind_specific_memory_keys_for_delegate_runs() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());

        let run_id = create_delegate_run("workflow", "finish hosted workflow").unwrap();
        mark_delegate_run_handed_off(&run_id, "delegated to host LLM", "handoff").unwrap();

        record_actual_tokens(&run_id, 42, "host-llm", Some("workflow final output")).unwrap();

        let workflow_key = format!("workflow_{}", &run_id[..8.min(run_id.len())]);
        let stored = memory_store()
            .scoped("project")
            .read(&workflow_key)
            .unwrap()
            .unwrap();
        assert_eq!(stored, "workflow final output");

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn record_actual_tokens_writes_structured_memory_records() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());

        let run_id = create_delegate_run("workflow", "ship release").unwrap();
        mark_delegate_run_handed_off(&run_id, "delegated to host LLM", "handoff").unwrap();

        record_actual_tokens(&run_id, 77, "host-llm", Some("workflow result")).unwrap();

        let scoped = memory_store().scoped("project");
        let by_run = scoped
            .read(&format!("run_records:{run_id}"))
            .unwrap()
            .unwrap();
        let by_kind = scoped
            .read(&format!("workflow_runs:{run_id}"))
            .unwrap()
            .unwrap();
        let latest = scoped.read("workflow_last").unwrap().unwrap();
        let record: serde_json::Value = serde_json::from_str(&by_run).unwrap();
        let record_kind: serde_json::Value = serde_json::from_str(&by_kind).unwrap();
        let latest_record: serde_json::Value = serde_json::from_str(&latest).unwrap();

        for value in [record, record_kind, latest_record] {
            assert_eq!(value["run_id"], run_id);
            assert_eq!(value["kind"], "workflow");
            assert_eq!(value["task"], "ship release");
            assert_eq!(value["model"], "host-llm");
            assert_eq!(value["tokens"], 77);
            assert_eq!(value["output"], "workflow result");
            assert_eq!(value["source"], "agent007_record_tokens");
            assert!(value["timestamp"].as_str().is_some());
        }

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn record_actual_tokens_uses_existing_output_when_output_arg_missing() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());

        let run_id = create_delegate_run("skill", "summarize findings").unwrap();
        mark_delegate_run_handed_off(&run_id, "delegated to host LLM", "handoff").unwrap();
        load_run_store()
            .write_text_artifact(&run_id, "output.txt", "captured output")
            .unwrap();

        record_actual_tokens(&run_id, 19, "host-llm", None).unwrap();

        let detail = load_run_store().load_run(&run_id).unwrap();
        assert_eq!(
            detail.metadata.output_preview.as_deref(),
            Some("captured output")
        );

        let key = format!("skill_{}", &run_id[..8.min(run_id.len())]);
        let memory = memory_store()
            .scoped("project")
            .read(&key)
            .unwrap()
            .unwrap();
        assert_eq!(memory, "captured output");

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn record_actual_tokens_skips_non_stale_finished_runs() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());

        let run_id = create_delegate_run("workflow", "already done").unwrap();
        load_run_store()
            .finish_run(&run_id, false, "actual failure")
            .unwrap();

        let message = record_actual_tokens(&run_id, 42, "host-llm", Some("ignored")).unwrap();
        assert!(message.contains("already finalized"));
        assert!(message.contains("failed"));

        let detail = load_run_store().load_run(&run_id).unwrap();
        assert!(matches!(
            detail.metadata.status,
            agent007_core::run_store::RunStatus::Failed
        ));
        assert_eq!(
            detail.metadata.output_preview.as_deref(),
            Some("actual failure")
        );

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn uuid_v4_has_correct_format() {
        let id = uuid_v4();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        // Version nibble must be '4'
        assert!(
            parts[2].starts_with('4'),
            "UUID version nibble must be 4, got: {}",
            id
        );
    }

    #[test]
    fn memory_list_on_nonexistent_scope_returns_empty() {
        let _guard = env_lock();
        // Use a temp AGENT007_HOME so we don't touch the real one
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        let keys = memory_list("nonexistent_scope").unwrap();
        assert!(keys.is_empty());
        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn memory_list_global_does_not_include_project_scope_entries() {
        let _guard = env_lock();
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let project_home = project.path().join(".agent007");
        std::fs::create_dir_all(&project_home).unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("AGENT007_HOME", &project_home);
        std::env::set_var("HOME", global.path());

        memory_write("project", "project_only", "project value").unwrap();
        memory_write("global", "global_only", "global value").unwrap();

        let keys = memory_list("global").unwrap();
        assert!(keys.contains(&"global_only".to_string()));
        assert!(!keys.iter().any(|key| key.contains("project_only")));
        assert!(!keys.iter().any(|key| key.starts_with("project:")));

        std::env::remove_var("AGENT007_HOME");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn record_feedback_entry_writes_learning_to_project_write_home() {
        let _guard = env_lock();
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let project_home = project.path().join(".agent007");
        std::fs::create_dir_all(&project_home).unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("AGENT007_HOME", &project_home);
        std::env::set_var("HOME", global.path());

        record_feedback_entry("host-llm", Some("/brainstorm"));

        let project_store = std::sync::Arc::new(agent007_memory::store::MemoryStore::new(
            project_home.join("memory"),
        ));
        let project_learning_keys = project_store.scoped("learning").list_keys().unwrap();
        assert!(
            !project_learning_keys.is_empty(),
            "expected learning feedback keys in project memory scope"
        );

        let global_store = std::sync::Arc::new(agent007_memory::store::MemoryStore::new(
            global.path().join(".agent007").join("memory"),
        ));
        let global_learning_keys = global_store
            .scoped("learning")
            .list_keys()
            .unwrap_or_default();
        assert!(
            global_learning_keys.is_empty(),
            "did not expect passive feedback writes in global learning scope"
        );

        std::env::remove_var("AGENT007_HOME");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn workflow_list_on_missing_dir_returns_empty() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        let names = workflow_list().unwrap();
        assert!(names.is_empty());
        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn skill_create_writes_file() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        let result = skill_create(
            "my skill",
            "/my-skill",
            "does something",
            "Do {{ args }}",
            "claude-sonnet-4-6",
        );
        assert!(result.is_ok(), "skill_create failed: {:?}", result.err());
        let path = tmp.path().join("skills").join("my_skill.md");
        assert!(path.exists(), "expected skill file at {}", path.display());
        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn workflow_hosted_roundtrip_completes_simple_workflow() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        write_workflow_fixture(
            tmp.path(),
            "simple",
            r#"
name = "Simple Workflow"

[[steps]]
id = "research"
agent = "Researcher"
prompt = "Research {{task}}"
output = "notes"
"#,
        );

        let started = workflow_hosted_start("simple", "ship feature").unwrap();
        let started: serde_json::Value = serde_json::from_str(&started).unwrap();
        let session = started["session"].as_str().unwrap().to_string();
        assert_eq!(started["progress"]["status"], "ready");
        assert_eq!(started["progress"]["ready_steps"][0]["id"], "research");

        let completed =
            workflow_hosted_submit_step(&session, "research", "notes v1", None).unwrap();
        let completed: serde_json::Value = serde_json::from_str(&completed).unwrap();
        assert_eq!(completed["progress"]["status"], "succeeded");
        assert_eq!(completed["workflow_state"]["outputs"]["notes"], "notes v1");

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn workflow_hosted_roundtrip_handles_approval_edit() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        write_workflow_fixture(
            tmp.path(),
            "approval-flow",
            r#"
name = "Approval Flow"

[[steps]]
id = "plan"
agent = "Architect"
prompt = "Plan {{task}}"
output = "plan"
requires_approval = true
"#,
        );

        let started = workflow_hosted_start("approval-flow", "ship feature").unwrap();
        let started: serde_json::Value = serde_json::from_str(&started).unwrap();
        let session = started["session"].as_str().unwrap().to_string();

        let waiting = workflow_hosted_submit_step(&session, "plan", "draft plan", None).unwrap();
        let waiting: serde_json::Value = serde_json::from_str(&waiting).unwrap();
        assert_eq!(waiting["progress"]["status"], "awaiting-approval");
        // approval_gate must be present with HUMAN_APPROVAL_REQUIRED so the host LLM stops
        assert_eq!(waiting["approval_gate"]["HUMAN_APPROVAL_REQUIRED"], true);
        assert_eq!(waiting["approval_gate"]["step_id"], "plan");
        assert_eq!(waiting["approval_gate"]["content"], "draft plan");
        // STOP_INSTRUCTIONS must be non-empty
        assert!(
            waiting["approval_gate"]["STOP_INSTRUCTIONS"]
                .as_array()
                .unwrap()
                .len()
                > 0
        );

        let approval =
            workflow_approve(&session, None, "edit", Some("approved plan".to_string())).unwrap();
        assert!(approval.contains("agent007_workflow_next"));

        let resumed = workflow_hosted_next(&session).unwrap();
        let resumed: serde_json::Value = serde_json::from_str(&resumed).unwrap();
        assert_eq!(resumed["progress"]["status"], "succeeded");
        assert_eq!(
            resumed["workflow_state"]["outputs"]["plan"],
            "approved plan"
        );

        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn workflow_hosted_parallel_submit_step_updates_are_atomic() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        write_workflow_fixture(
            tmp.path(),
            "code-review",
            r#"
name = "Code Review"

[[steps]]
id = "security-review"
agent = "SecurityReviewer"
prompt = "Security review {{task}}"
output = "security_findings"

[[steps]]
id = "performance-review"
agent = "PerformanceEngineer"
prompt = "Performance review {{task}}"
output = "performance_findings"

[[steps]]
id = "quality-review"
agent = "CodeReviewer"
prompt = "Quality review {{task}}"
output = "quality_findings"

[[steps]]
id = "synthesize"
agent = "CodeReviewer"
depends_on = ["security-review", "performance-review", "quality-review"]
prompt = "Synthesize {{security_findings}} {{performance_findings}} {{quality_findings}}"
output = "review_report"
"#,
        );

        let started = workflow_hosted_start("code-review", "race repro").unwrap();
        let started: serde_json::Value = serde_json::from_str(&started).unwrap();
        let session = started["session"].as_str().unwrap().to_string();

        let mut handles = Vec::new();
        for (step, output) in [
            ("security-review", "security findings"),
            ("performance-review", "performance findings"),
            ("quality-review", "quality findings"),
        ] {
            let session = session.clone();
            let step = step.to_string();
            let output = output.to_string();
            handles.push(thread::spawn(move || {
                workflow_hosted_submit_step(&session, &step, &output, None)
            }));
        }
        for handle in handles {
            let result = handle.join().expect("submit thread panicked");
            assert!(result.is_ok(), "parallel submit failed: {:?}", result.err());
        }

        let status = workflow_hosted_status(&session).unwrap();
        let status: serde_json::Value = serde_json::from_str(&status).unwrap();
        let completed = status["workflow_state"]["completed_steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry.as_str())
            .collect::<Vec<_>>();
        assert_eq!(completed.len(), 3);
        assert!(completed.contains(&"security-review"));
        assert!(completed.contains(&"performance-review"));
        assert!(completed.contains(&"quality-review"));
        assert_eq!(status["progress"]["status"], "awaiting-outputs");
        assert_eq!(status["progress"]["running_steps"][0], "synthesize");

        let finished =
            workflow_hosted_submit_step(&session, "synthesize", "final report", None).unwrap();
        let finished: serde_json::Value = serde_json::from_str(&finished).unwrap();
        assert_eq!(finished["progress"]["status"], "succeeded");
        assert_eq!(
            finished["workflow_state"]["outputs"]["review_report"],
            "final report"
        );

        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn workflow_run_injects_memory_placeholders_in_standalone_mode() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        std::env::set_var("AGENT007_DRY_RUN", "1");
        write_workflow_fixture(
            tmp.path(),
            "code-review",
            r#"
name = "Code Review"

[[steps]]
id = "quality-review"
agent = "CodeReviewer"
prompt = """
Review {{task}}
Project notes:
{{memory.project}}
Prior findings:
{{rag_context}}
"""
output = "quality_findings"
"#,
        );

        let report = workflow_run(&Config::default(), "code-review", "review current diff")
            .await
            .unwrap();
        assert!(report.contains("Workflow: Code Review"));
        assert!(report.contains("quality_findings"));

        std::env::remove_var("AGENT007_DRY_RUN");
        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn workflow_run_returns_inline_approval_instructions() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        std::env::set_var("AGENT007_DRY_RUN", "1");
        write_workflow_fixture(
            tmp.path(),
            "approval-flow",
            r#"
name = "Approval Flow"

[[steps]]
id = "plan"
agent = "Architect"
prompt = "Plan {{task}}"
output = "plan"
requires_approval = true
"#,
        );

        let report = workflow_run(&Config::default(), "approval-flow", "ship feature")
            .await
            .unwrap();
        assert!(report.contains("waiting for approval on step 'plan'"));
        assert!(report.contains("Pending approval content:"));
        assert!(report.contains("agent007_workflow_approve"));
        assert!(report.contains("agent007_workflow_resume"));

        std::env::remove_var("AGENT007_DRY_RUN");
        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn workflow_run_starts_hosted_session_in_hosted_mcp_mode() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let _home = EnvVarGuard::set("AGENT007_HOME", tmp.path());
        let _dry_run = EnvVarGuard::unset("AGENT007_DRY_RUN");
        let _openai = EnvVarGuard::unset("OPENAI_API_KEY");
        let _anthropic = EnvVarGuard::unset("ANTHROPIC_API_KEY");
        write_workflow_fixture(
            tmp.path(),
            "hosted-flow",
            r#"
name = "Hosted Flow"

[[steps]]
id = "plan"
agent = "Architect"
prompt = "Plan {{task}}"
output = "plan"
"#,
        );

        let report = workflow_run(&Config::default(), "hosted-flow", "ship feature")
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(payload["mode"], "hosted-mcp");
        assert_eq!(payload["request"]["workflow"], "hosted-flow");
        assert_eq!(payload["progress"]["status"], "ready");
        assert_eq!(payload["progress"]["ready_steps"][0]["id"], "plan");
        assert!(payload["session"].as_str().unwrap_or("").len() > 10);
        assert!(payload["execution_instructions"]["steps"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("workflow_submit_step")));
    }

    #[test]
    fn dashboard_port_roundtrips_from_project_memory() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        persist_dashboard_port(8123);
        assert_eq!(read_dashboard_port(), Some(8123));
        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn persist_dashboard_port_registers_port_globally_per_project() {
        let _guard = env_lock();
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let project_home = project.path().join(".agent007");
        std::fs::create_dir_all(&project_home).unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("AGENT007_HOME", &project_home);
        std::env::set_var("HOME", global.path());

        persist_dashboard_port(8124);
        assert_eq!(
            read_registered_dashboard_port_for_current_project(),
            Some(8124)
        );

        std::env::remove_var("AGENT007_HOME");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[tokio::test]
    async fn dashboard_port_is_live_detects_bound_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(dashboard_port_is_live(port).await);
    }

    #[test]
    fn compact_output_tool_records_artifacts() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        let config = Config::default();
        let output = compact_output_tool(
            &config,
            "cargo test",
            "warning: one\ntest auth::works ... FAILED\ntest result: FAILED. 9 passed; 1 failed",
            "compact",
        )
        .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&output).unwrap();
        let run_id = payload["run_id"].as_str().unwrap();
        let store = load_run_store();
        let detail = store.load_run(run_id).unwrap();
        assert!(detail
            .artifacts
            .contains(&"compact-output.json".to_string()));
        assert!(detail.artifacts.contains(&"raw-output.txt".to_string()));
        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn repo_brain_refresh_tool_persists_project_memory() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        let config = Config::default();
        let output = repo_brain_refresh_tool(&config).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(payload["memory_key"], "project/repo_brain");
        let stored = memory_read("project", "repo_brain").unwrap();
        assert!(stored.as_deref().unwrap_or("").contains("Repo Brain"));
        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn build_memory_context_reads_project_user_and_global_scopes_from_correct_homes() {
        let _guard = env_lock();
        let project_home = tempfile::tempdir().unwrap();
        let global_home = tempfile::tempdir().unwrap();
        let original_home = std::env::var("HOME").ok();

        std::env::set_var("AGENT007_HOME", project_home.path());
        std::env::set_var("HOME", global_home.path());
        std::env::set_var("AGENT007_INCLUDE_SHARED_MEMORY", "1");

        memory_write("project", "project_note", "project-local memory").unwrap();
        memory_write("project", "repo_brain", "repo brain summary").unwrap();
        memory_write("user", "user_note", "user-shared memory").unwrap();
        memory_write("global", "global_note", "global-shared memory").unwrap();

        let context = build_memory_context("project user global");

        assert!(context.project.contains("project-local memory"));
        assert!(context.repo_brain.contains("repo brain summary"));
        assert!(context.user.contains("user-shared memory"));
        assert!(context.global.contains("global-shared memory"));
        assert!(context.rag.contains("[project/project_note]"));
        assert!(context.rag.contains("[user/user_note]"));
        assert!(context.rag.contains("[global/global_note]"));

        std::env::remove_var("AGENT007_HOME");
        std::env::remove_var("AGENT007_INCLUDE_SHARED_MEMORY");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[tokio::test]
    async fn hosted_task_run_is_closed_immediately_and_waits_for_host_report() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());

        let response = run_task(&Config::default(), "ship hosted task".to_string())
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&response).unwrap();
        let run_id = payload["run_id"].as_str().unwrap();

        let detail = load_run_store().load_run(run_id).unwrap();
        assert_eq!(
            detail.metadata.status,
            agent007_core::run_store::RunStatus::Succeeded
        );
        assert_eq!(
            detail.metadata.output_preview.as_deref(),
            Some("delegated to host LLM")
        );
        let delegate_state =
            hosted_delegate_state(&load_run_store(), run_id).expect("delegate state should exist");
        assert!(delegate_state.awaiting_host_report);

        std::env::remove_var("AGENT007_HOME");
    }

    #[tokio::test]
    async fn missing_skill_lookup_does_not_create_stuck_run() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        std::env::set_var("AGENT007_DRY_RUN", "1");

        let result = run_skill_mcp(
            &Config::default(),
            "/definitely-missing-skill".to_string(),
            "arg".to_string(),
        )
        .await;
        assert!(result.is_err(), "expected missing skill error");

        let runs = load_run_store().list_runs(10).unwrap();
        assert!(
            runs.is_empty(),
            "missing skill should fail before run creation"
        );

        std::env::remove_var("AGENT007_DRY_RUN");
        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn config_show_returns_nonempty_string() {
        let config = Config::default();
        let text = config_show(&config);
        assert!(!text.is_empty());
    }

    #[test]
    fn health_check_returns_nonempty_string() {
        let config = Config::default();
        let text = health_check(&config);
        assert!(text.contains("agent007 health"));
    }

    #[test]
    fn zone_check_allowed_path() {
        let config = Config::default(); // no zones configured → everything unrestricted
        let result = zone_check(&config, "src/main.rs", "read").unwrap();
        assert!(
            result.contains("ALLOWED"),
            "expected ALLOWED, got: {}",
            result
        );
    }

    #[test]
    fn zone_check_denied_path() {
        let mut config = Config::default();
        config.zones.forbidden = vec!["secrets/".to_string()];
        let result = zone_check(&config, "secrets/token", "read").unwrap();
        assert!(
            result.contains("DENIED"),
            "expected DENIED, got: {}",
            result
        );
    }
}
