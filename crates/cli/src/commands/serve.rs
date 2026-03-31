use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, Content, Implementation,
        InitializeRequestParams, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
    transport::io::stdio,
};
use serde_json::Map;

use crate::config::Config;
use super::run::{
    agent007_global_home, agent007_home, agent007_project_home, build_stack, runtime_mode_label,
    selected_runtime_model, selected_runtime_provider, standalone_mode_available,
};
use super::skill::SkillSummary;

use agent007_core::dispatcher::LocalDispatcher;
use agent007_core::events::AgentEvent;
use agent007_core::types::PromptRef;
use agent007_learning::LearningDispatcher;

/// MCP server that exposes agent007 tools to Claude Code (or any MCP client).
pub struct Agent007Server {
    config: Arc<Config>,
    dispatcher: Option<Arc<LocalDispatcher>>,
    learning_dispatcher: Option<Arc<LearningDispatcher>>,
}

impl Agent007Server {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config, dispatcher: None, learning_dispatcher: None }
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

    async fn publish_task_assigned(&self, agent_id: &agent007_core::types::AgentId, task: &agent007_core::Task) {
        if let Some(d) = &self.dispatcher {
            use agent007_core::dispatcher::Dispatcher;
            let _ = d.publish(agent007_core::AgentEvent::TaskAssigned {
                agent_id: agent_id.clone(),
                task: task.clone(),
            }).await;
        }
    }

    async fn publish_task_completed(&self, agent_id: &agent007_core::types::AgentId, task_id: uuid::Uuid, output: &str) {
        if let Some(d) = &self.dispatcher {
            use agent007_core::dispatcher::Dispatcher;
            let _ = d.publish(agent007_core::AgentEvent::TaskCompleted {
                agent_id: agent_id.clone(),
                result: agent007_core::TaskResult::success(task_id, output.chars().take(200).collect()),
            }).await;
        }
    }

    async fn publish_model_request(&self, token_estimate: usize) {
        if let Some(d) = &self.dispatcher {
            use agent007_core::dispatcher::Dispatcher;
            let _ = d.publish(agent007_core::AgentEvent::ModelRequest {
                provider: selected_runtime_provider(&self.config)
                    .unwrap_or_else(|| "hosted-mcp".to_string()),
                prompt_ref: agent007_core::types::PromptRef::new(),
                token_estimate,
            }).await;
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
                 and the model name.",
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
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .build();
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
                Ok(output) => {
                    let token_est = output.len() / 4;
                    self.publish_model_request(token_est).await;
                    self.publish_task_completed(&aid, task_id, &output).await;
                    return Ok(CallToolResult::success(vec![Content::text(output)]));
                }
                Err(e) => {
                    return Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))]));
                }
            }
        }

        if let Some(workflow) = dynamic_workflow_tool(request.name.as_ref()) {
            let task = extract_string(request.arguments.as_ref(), "task")?;
            let aid = agent007_core::types::AgentId::new();
            let core_task = agent007_core::Task::new(&format!("workflow:{}", workflow.workflow_ref));
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
                    return Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))]));
                }
            }
        }

        match request.name.as_ref() {
            // ── existing 5 ────────────────────────────────────────────────
            "agent007_help" => {
                let topic = optional_string(request.arguments.as_ref(), "topic");
                Ok(CallToolResult::success(vec![Content::text(agent007_help(topic.as_deref()))]))
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
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }
            "agent007_skill_list" => {
                match list_available_skills() {
                    Ok(skills) => {
                        Ok(CallToolResult::success(vec![Content::text(format_skills(&skills))]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }
            "agent007_skill_run" => {
                let trigger = extract_string(request.arguments.as_ref(), "trigger")?;
                let args = string_or_default(request.arguments.as_ref(), "args", "");
                let aid = agent007_core::types::AgentId::new();
                let core_task = agent007_core::Task::new(&format!("skill:{trigger}"));
                let task_id = core_task.id;
                self.publish_task_assigned(&aid, &core_task).await;
                match run_skill_mcp(&self.config, trigger, args).await {
                    Ok(output) => {
                        let token_est = output.len() / 4;
                        self.publish_model_request(token_est).await;
                        self.publish_task_completed(&aid, task_id, &output).await;
                        Ok(CallToolResult::success(vec![Content::text(output)]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }
            "agent007_persona_list" => {
                let personas_dir = agent007_home().join("personas");
                let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
                    .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
                use agent007_core::PersonaProvider;
                let personas = registry.list();
                let text = if personas.is_empty() {
                    "No personas available.".to_string()
                } else {
                    personas
                        .iter()
                        .map(|p| format!("• {} [{}] — {}", p.name, p.preferred_model, p.description))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            "agent007_persona_show" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let personas_dir = agent007_home().join("personas");
                let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
                    .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
                use agent007_core::PersonaProvider;
                match registry.get(&name) {
                    Some(spec) => {
                        let tools = if spec.allowed_tools.is_empty() {
                            "none".to_string()
                        } else {
                            spec.allowed_tools.join(", ")
                        };
                        let text = format!(
                            "Name: {}\nModel: {}\nDescription: {}\nAllowed tools: {}\n\nSystem prompt:\n{}",
                            spec.name, spec.preferred_model, spec.description, tools, spec.system_prompt
                        );
                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                    None => Ok(CallToolResult::error(vec![Content::text(
                        format!("Persona '{}' not found.", name)
                    )])),
                }
            }

            // ── new tools ─────────────────────────────────────────────────

            // 1. Memory read
            "agent007_memory_read" => {
                let scope = extract_string(request.arguments.as_ref(), "scope")?;
                let key   = extract_string(request.arguments.as_ref(), "key")?;
                match memory_read(&scope, &key) {
                    Ok(Some(val)) => Ok(CallToolResult::success(vec![Content::text(val)])),
                    Ok(None)      => Ok(CallToolResult::success(vec![Content::text(
                        format!("Key '{}' not found in scope '{}'.", key, scope)
                    )])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 2. Memory write
            "agent007_memory_write" => {
                let scope = extract_string(request.arguments.as_ref(), "scope")?;
                let key   = extract_string(request.arguments.as_ref(), "key")?;
                let value = extract_string(request.arguments.as_ref(), "value")?;
                match memory_write(&scope, &key, &value) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text(
                        format!("Written key '{}' in scope '{}'.", key, scope)
                    )])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
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
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 4. Record actual tokens
            "agent007_record_tokens" => {
                let run_id = extract_string(request.arguments.as_ref(), "run_id")?;
                let tokens = request.arguments.as_ref()
                    .and_then(|a| a.get("tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let model = extract_string(request.arguments.as_ref(), "model")?;
                match record_actual_tokens(&run_id, tokens, &model) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text(
                        format!("Recorded {} tokens for run '{}' (model: {}).", tokens, run_id, model)
                    )])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 5. Workflow list
            "agent007_workflow_list" => {
                match workflow_list() {
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
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

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
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
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
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            "agent007_workflow_approve" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                let step = optional_string(request.arguments.as_ref(), "step");
                let decision = extract_string(request.arguments.as_ref(), "decision")?;
                let content = optional_string(request.arguments.as_ref(), "content");
                match workflow_approve(&session, step, &decision, content) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            "agent007_workflow_start" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                let task = extract_string(request.arguments.as_ref(), "task")?;
                match workflow_hosted_start(&name, &task) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            "agent007_workflow_next" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                match workflow_hosted_next(&session) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            "agent007_workflow_submit_step" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                let step = extract_string(request.arguments.as_ref(), "step")?;
                let output = extract_string(request.arguments.as_ref(), "output")?;
                match workflow_hosted_submit_step(&session, &step, &output) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            "agent007_workflow_status" => {
                let session = extract_string(request.arguments.as_ref(), "session")?;
                match workflow_hosted_status(&session) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 6. Git status
            "agent007_git_status" => {
                match git_run(&["status"]) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 7. Git diff
            "agent007_git_diff" => {
                // Show both unstaged and staged diffs
                let unstaged = git_run(&["diff"]).unwrap_or_default();
                let staged   = git_run(&["diff", "--staged"]).unwrap_or_default();
                let combined = format!(
                    "=== Unstaged ===\n{}\n=== Staged ===\n{}",
                    if unstaged.is_empty() { "(none)" } else { &unstaged },
                    if staged.is_empty()   { "(none)" } else { &staged   },
                );
                Ok(CallToolResult::success(vec![Content::text(combined)]))
            }

            // 8. Git log
            "agent007_git_log" => {
                let n = number_or_default(request.arguments.as_ref(), "n", 10);
                let n_str = n.to_string();
                match git_run(&["log", "--oneline", &format!("-{}", n_str)]) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 9. Git commit
            "agent007_git_commit" => {
                let message = extract_string(request.arguments.as_ref(), "message")?;
                match git_run(&["commit", "-m", &message]) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 10. Persona switch
            "agent007_persona_switch" => {
                let name = extract_string(request.arguments.as_ref(), "name")?;
                // Validate the persona exists
                let personas_dir = agent007_home().join("personas");
                let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
                    .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
                use agent007_core::PersonaProvider;
                if registry.get(&name).is_none() {
                    return Ok(CallToolResult::error(vec![Content::text(
                        format!("Persona '{}' not found.", name)
                    )]));
                }
                // Store in memory under scope "user", key "active_persona"
                match memory_write("user", "active_persona", &name) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text(
                        format!("Active persona switched to '{}'.", name)
                    )])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 11. Zone check
            "agent007_zone_check" => {
                let path_str  = extract_string(request.arguments.as_ref(), "path")?;
                let operation = extract_string(request.arguments.as_ref(), "operation")?;
                match zone_check(&self.config, &path_str, &operation) {
                    Ok(result) => Ok(CallToolResult::success(vec![Content::text(result)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
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
                match task_submit(&self.config, task_str, if persona.is_empty() { None } else { Some(persona) }).await {
                    Ok(output) => {
                        self.publish_task_completed(&aid, task_id, &output).await;
                        Ok(CallToolResult::success(vec![Content::text(output)]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 13. Skill create
            "agent007_skill_create" => {
                let name        = extract_string(request.arguments.as_ref(), "name")?;
                let trigger     = extract_string(request.arguments.as_ref(), "trigger")?;
                let description = extract_string(request.arguments.as_ref(), "description")?;
                let template    = extract_string(request.arguments.as_ref(), "template")?;
                let default_model = self.config.models.default_provider();
                let model = string_or_default(
                    request.arguments.as_ref(),
                    "model",
                    default_model.as_str(),
                );
                match skill_create(&name, &trigger, &description, &template, &model) {
                    Ok(path) => Ok(CallToolResult::success(vec![Content::text(
                        format!("Skill '{}' created at {}.", name, path)
                    )])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
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
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
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
                        let description = extract_string(request.arguments.as_ref(), "description")?;
                        let system_prompt = extract_string(request.arguments.as_ref(), "system_prompt")?;
                        let default_provider = self.config.models.default_provider();
                        let preferred_model = string_or_default(
                            request.arguments.as_ref(),
                            "preferred_model",
                            default_provider.as_str(),
                        );
                        let allowed_tools = request.arguments.as_ref()
                            .and_then(|a| a.get("allowed_tools"))
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
                            .unwrap_or_default();
                        match agent_save(&name, &description, &system_prompt, &preferred_model, &allowed_tools) {
                            Ok(path) => Ok(CallToolResult::success(vec![Content::text(
                                format!("Agent '{}' saved to {}. It is now available for workflows and orchestration.", name, path)
                            )])),
                            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                        }
                    }
                    other => Ok(CallToolResult::error(vec![Content::text(
                        format!("Unknown action '{}'. Use 'catalog' or 'save'.", other)
                    )])),
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
                        let description = extract_string(request.arguments.as_ref(), "description")?;
                        let template = extract_string(request.arguments.as_ref(), "template")?;
                        let default_model = self.config.models.default_provider();
                        let model = string_or_default(
                            request.arguments.as_ref(),
                            "model",
                            default_model.as_str(),
                        );
                        match skill_create(&name, &trigger, &description, &template, &model) {
                            Ok(path) => Ok(CallToolResult::success(vec![Content::text(
                                format!("Skill '{}' created at {}. Trigger: {}", name, path, trigger)
                            )])),
                            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                        }
                    }
                    other => Ok(CallToolResult::error(vec![Content::text(
                        format!("Unknown action '{}'. Use 'templates' or 'save'.", other)
                    )])),
                }
            }

            // 19. Downstream MCP tool list
            "agent007_mcp_tools_list" => {
                match mcp_tools_list(&self.config).await {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

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
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 21. Run history
            "agent007_run_history" => {
                let limit = number_or_default(request.arguments.as_ref(), "limit", 10) as usize;
                match run_history(limit) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 22. Run show
            "agent007_run_show" => {
                let id = extract_string(request.arguments.as_ref(), "id")?;
                match run_show(&id) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            "agent007_compact_output" => {
                let command = extract_string(request.arguments.as_ref(), "command")?;
                let output = extract_string(request.arguments.as_ref(), "output")?;
                let level = string_or_default(request.arguments.as_ref(), "level", "compact");
                match compact_output_tool(&self.config, &command, &output, &level) {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            "agent007_context_compile" => {
                let task = extract_string(request.arguments.as_ref(), "task")?;
                let max_files = number_or_default(request.arguments.as_ref(), "max_files", 8) as usize;
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
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            "agent007_repo_brain_refresh" => match repo_brain_refresh_tool(&self.config) {
                Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
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
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
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

fn optional_string(
    args: Option<&Map<String, serde_json::Value>>,
    key: &str,
) -> Option<String> {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
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
        for skill in loader
            .load_all()
            .map_err(|e| anyhow::anyhow!("failed to load skills from {}: {}", skills_dir.display(), e))?
        {
            skills
                .entry(skill.trigger().to_string())
                .or_insert(skill);
        }
    }

    Ok(skills.into_values().collect())
}

fn list_available_skills() -> Result<Vec<SkillSummary>> {
    Ok(load_available_skills()?
        .into_iter()
        .map(|skill| SkillSummary {
            name: skill.name().to_string(),
            description: skill.frontmatter.description.clone(),
            trigger: skill.trigger().to_string(),
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
    dynamic_skill_catalog()
        .into_iter()
        .find(|tool| tool.tool_name == name || tool.legacy_tool_names.iter().any(|alias| alias == name))
}

fn load_available_workflows() -> Result<Vec<(String, agent007_workflows::WorkflowDef)>> {
    let mut workflows = BTreeMap::new();

    for workflows_dir in configured_workflow_dirs() {
        if !workflows_dir.exists() {
            continue;
        }

        let loader = agent007_workflows::WorkflowLoader::new(workflows_dir.clone());
        for workflow_ref in loader
            .list_names()
            .map_err(|e| anyhow::anyhow!("failed to list workflows from {}: {}", workflows_dir.display(), e))?
        {
            if workflows.contains_key(&workflow_ref) {
                continue;
            }
            let def = loader
                .load_named(&workflow_ref)
                .map_err(|e| anyhow::anyhow!("failed to load workflow '{}': {}", workflow_ref, e))?;
            workflows.insert(workflow_ref, def);
        }
    }

    Ok(workflows.into_iter().collect())
}

fn dynamic_workflow_catalog() -> Vec<DynamicWorkflowTool> {
    let mut tools = BTreeMap::new();

    if let Ok(workflows) = load_available_workflows() {
        for (workflow_ref, def) in workflows {
            let tool_name = format!("agent007_workflow_{}", sanitize_tool_component(&workflow_ref));
            tools.entry(tool_name.clone()).or_insert(DynamicWorkflowTool {
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
        ("agent007_run", "Run a general task through agent007"),
        ("agent007_help", "Show this catalog and invocation guidance"),
        ("agent007_skill_list", "List installed skills"),
        ("agent007_skill_run", "Run a skill by trigger"),
        ("agent007_workflow_list", "List installed workflows"),
        ("agent007_workflow_run", "Run a workflow by name"),
        ("agent007_workflow_start", "Start a hosted MCP workflow session"),
        ("agent007_workflow_next", "Get the next hosted workflow steps"),
        ("agent007_workflow_submit_step", "Submit hosted workflow step output"),
        ("agent007_workflow_status", "Inspect hosted workflow state"),
    ];

    let mut lines = Vec::new();

    if matches!(topic, "overview" | "tools") {
        lines.push("agent007 MCP Catalog".to_string());
        lines.push(String::new());
        lines.push("How to invoke from Codex: ask Codex in plain language to call a named MCP tool.".to_string());
        lines.push("Examples:".to_string());
        lines.push("- Use the MCP tool agent007_workflow_tdd with task \"build login flow with tests\".".to_string());
        lines.push("- Use the MCP tool agent007_skill_write_tests with args \"write tests for auth middleware\".".to_string());
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
                let description = workflow.description.unwrap_or_else(|| "No description".to_string());
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

    Ok(ApprovalDecision { decision: kind, content })
}

fn format_skills(skills: &[SkillSummary]) -> String {
    if skills.is_empty() {
        return "No skills loaded. Add skills to ~/.agent007/skills/".to_string();
    }
    skills
        .iter()
        .map(|s| format!("• {} — {}", s.trigger, s.description))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── task helpers ─────────────────────────────────────────────────────────────

fn load_run_store() -> agent007_core::RunStore {
    agent007_core::RunStore::new(agent007_home().join("sessions"))
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
    Ok((stack, run.id))
}

fn create_delegate_run(kind: &str, task: &str) -> Result<String> {
    let run = load_run_store().create_run(kind, task, "hosted-mcp", None)?;
    Ok(run.id)
}

/// Returns the model label for hosted-mcp runs.
/// Checks AGENT007_HOST_MODEL env var first; falls back to "hosted-mcp".
fn hosted_model_label() -> String {
    std::env::var("AGENT007_HOST_MODEL").unwrap_or_else(|_| "hosted-mcp".to_string())
}

/// Appends an estimated ModelRequest event so token counts appear in the dashboard.
/// Estimate: 1 token ≈ 4 characters (GPT/Claude tokenizer rule of thumb).
/// `model` is the declared model (from skill frontmatter or env var).
/// Appends an exact ModelRequest event with the actual token count reported by the host LLM.
/// Called via the `agent007_record_tokens` MCP tool after the host finishes its LLM work.
fn record_actual_tokens(run_id: &str, tokens: usize, model: &str) -> Result<()> {
    load_run_store().append_event(
        run_id,
        &AgentEvent::ModelRequest {
            provider: model.to_string(),
            prompt_ref: PromptRef::new(),
            token_estimate: tokens,
        },
    ).map_err(|e| anyhow::anyhow!("{}", e))?;
    write_statusline();
    Ok(())
}

fn record_estimated_tokens(run_id: &str, prompt_chars: usize, model: &str) {
    let token_estimate = (prompt_chars / 4).max(1);
    let _ = load_run_store().append_event(
        run_id,
        &AgentEvent::ModelRequest {
            provider: model.to_string(),
            prompt_ref: PromptRef::new(),
            token_estimate,
        },
    );
    write_statusline();
}

/// Cost per token in USD (blended input+output at Claude Sonnet rates).
const STATUSLINE_PRICE_PER_TOKEN: f64 = 0.000_006;

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
    let succeeded = runs.iter().filter(|r| matches!(r.status, RunStatus::Succeeded)).count();
    let failed    = runs.iter().filter(|r| matches!(r.status, RunStatus::Failed)).count();
    let running   = runs.iter().filter(|r| matches!(r.status, RunStatus::Running | RunStatus::AwaitingApproval)).count();

    // ── Token + model scan (20 most recent runs) ──────────────────────────────
    let mut total_tokens: u64 = 0;
    let mut last_model = std::env::var("AGENT007_HOST_MODEL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "hosted-mcp".to_string());

    for run in runs.iter().take(20) {
        if let Ok(detail) = store.load_run(&run.id) {
            for entry in &detail.entries {
                if entry.kind != "agent-event" { continue; }
                if let Ok(AgentEvent::ModelRequest { token_estimate, provider, .. }) =
                    serde_json::from_value::<AgentEvent>(entry.payload.clone())
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
        .strip_prefix("claude-").unwrap_or(&last_model)
        .to_string();

    // ── Last finished task ────────────────────────────────────────────────────
    let last_finished = runs.iter()
        .find(|r| matches!(r.status, RunStatus::Succeeded | RunStatus::Failed));

    let last_segment = if let Some(run) = last_finished {
        let icon = if matches!(run.status, RunStatus::Succeeded) { "✓" } else { "✗" };
        let kind_badge = match run.kind.as_str() {
            "skill"     => "skill",
            "task"      => "task",
            "workflow"  => "wf",
            "task-submit" => "task",
            other       => other,
        };
        let desc = run.task.chars().take(32).collect::<String>();
        let ellipsis = if run.task.chars().count() > 32 { "…" } else { "" };
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
        ["user", "project", "skills", ""].iter()
            .filter_map(|ns| mem_store.scoped(ns).list_keys().ok())
            .map(|ks| ks.len())
            .sum()
    };

    // ── Dashboard port ────────────────────────────────────────────────────────
    let dash_segment = {
        let port_path = agent007_home().join("memory").join("project").join("dashboard_port.md");
        std::fs::read_to_string(port_path)
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .map(|p| format!("⬡ :{p}"))
            .unwrap_or_else(|| "⬡ offline".to_string())
    };

    // ── Running indicator ─────────────────────────────────────────────────────
    let run_stats = if running > 0 {
        format!("✓{succeeded} ✗{failed} ↺{running}")
    } else {
        format!("✓{succeeded} ✗{failed}")
    };

    let line = format!(
        "◈ agent007  ◎ {model_short}  {run_stats}  ⚡ {tok_display} · ~{cost_display}  {last_segment}  🗝 {mem_count} mem  {dash_segment}"
    );

    let path = agent007_home().join("statusline");
    let _ = std::fs::write(&path, &line);
}

fn create_recorded_utility_run(config: &Config, kind: &str, task: &str) -> Result<String> {
    let provider = selected_runtime_provider(config);
    let run = load_run_store().create_run(
        kind,
        task,
        runtime_mode_label(config),
        provider.as_deref(),
    )?;
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
                let _ = stack.run_store.finish_run(&run_id, false, error.to_string());
                Err(error.into())
            }
        }
    } else {
        let run_id = create_delegate_run("task", &task)?;
        let task_escaped = task.replace('"', "\\\"");
        let output = format!(
            "{{\"mode\":\"hosted-mcp\",\"task\":\"{task_escaped}\",\"run_id\":\"{run_id}\",\"instructions\":\
             \"No standalone provider is configured inside agent007. Execute this task directly using your host LLM capabilities. \
             Use agent007_memory_write to persist results, agent007_workflow_plan to decompose \
             complex tasks into multi-agent workflows. \
             IMPORTANT: After you finish, call agent007_record_tokens with run_id={run_id}, \
             the actual total tokens you used (input+output), and your model name — this records real token counts in the dashboard.\"}}"
        );
        record_estimated_tokens(&run_id, task.len(), &hosted_model_label());
        let _ = load_run_store().finish_run(&run_id, true, &output);
        Ok(output)
    }
}

async fn run_skill_mcp(config: &Config, trigger: String, args: String) -> Result<String> {
    if standalone_mode_available(config) {
        let trace_task = format!("skill:{} {}", trigger, args);
        let (stack, run_id) = create_traced_stack(config, "skill", &trace_task).await?;
        let skill = find_skill(&trigger)?;
        match stack.skill_executor.execute(&skill, &args).await {
            Ok(output) => {
                let _ = stack.run_store.finish_run(&run_id, true, &output);
                Ok(output)
            }
            Err(error) => {
                let _ = stack.run_store.finish_run(&run_id, false, error.to_string());
                Err(error.into())
            }
        }
    } else {
        let skill = find_skill(&trigger)?;

        let rendered = skill.template()
            .replace("{{args}}", &args)
            .replace("{{ args }}", &args)
            .replace("{{task}}", &args)
            .replace("{{ task }}", &args);

        let run_id = create_delegate_run("skill", &format!("{trigger} {args}"))?;
        let output = format!(
            "[HOSTED MCP MODE — execute the following as the host LLM]\n\n\
             Skill: {} ({})\n\
             Run ID: {}\n\n\
             ---\n\n\
             {}\n\n\
             ---\n\
             After completing this skill, call agent007_record_tokens with run_id={}, \
             the actual total tokens you used (input+output), and your model name.\n",
            skill.name(), trigger, run_id, rendered, run_id,
        );
        // Use the model declared in the skill's frontmatter (e.g. "claude-sonnet-4-6").
        // If the user set AGENT007_HOST_MODEL it overrides.
        let model_label = std::env::var("AGENT007_HOST_MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| skill.model().to_string());
        record_estimated_tokens(&run_id, rendered.len(), &model_label);
        let _ = load_run_store().finish_run(&run_id, true, &output);
        Ok(output)
    }
}

// ── memory helpers ────────────────────────────────────────────────────────────

fn memory_store() -> Arc<agent007_memory::store::MemoryStore> {
    let memory_dir = agent007_home().join("memory");
    Arc::new(agent007_memory::store::MemoryStore::new(memory_dir))
}

fn memory_read(scope: &str, key: &str) -> Result<Option<String>> {
    let store = memory_store();
    let scoped = store.scoped(scope);
    scoped.read(key).map_err(|e| anyhow::anyhow!("{}", e))
}

fn memory_write(scope: &str, key: &str, value: &str) -> Result<()> {
    let store = memory_store();
    let scoped = store.scoped(scope);
    scoped.write(key, value).map_err(|e| anyhow::anyhow!("{}", e))
}

fn memory_list(scope: &str) -> Result<Vec<String>> {
    let store = memory_store();
    let effective_scope = if scope.is_empty() || scope == "global" { "" } else { scope };
    store.scoped(effective_scope)
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
        let run_id = create_delegate_run("workflow", &format!("{name}: {task}"))?;
        let output = workflow_plan(config, name, task).await?;
        let _ = load_run_store().finish_run(&run_id, true, &output);
        return Ok(output);
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
    let request: agent007_workflows::WorkflowRunRequest = store
        .read_json_artifact(session, "workflow-request.json")?;
    let state: agent007_workflows::WorkflowRunState = store
        .read_json_artifact(session, "workflow-state.json")?;
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
    let store = load_run_store();
    let mut state: agent007_workflows::WorkflowRunState = store
        .read_json_artifact(session, "workflow-state.json")?;
    let step_id = step
        .or_else(|| state.pending_approval.as_ref().map(|pending| pending.step_id.clone()))
        .ok_or_else(|| anyhow::anyhow!("no pending approval found in session {}", session))?;
    let decision = parse_approval_decision(decision, content)?;
    state.record_approval_decision(&step_id, decision);
    store.write_json_artifact(session, "workflow-state.json", &state)?;
    Ok(format!(
        "Recorded approval decision for step '{}' in session {}. Continue with agent007_workflow_next, agent007_workflow_status, or `agent007 workflow resume --session {}`.",
        step_id, session, session,
    ))
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

fn workflow_persona_provider() -> Arc<dyn agent007_core::PersonaProvider> {
    let personas_dir = agent007_home().join("personas");
    let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
        .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
    let provider: Arc<dyn agent007_core::PersonaProvider> = Arc::new(registry);
    provider
}

fn hosted_workflow_engine() -> agent007_workflows::HostedWorkflowEngine {
    agent007_workflows::HostedWorkflowEngine::new(workflow_persona_provider())
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
    let request: agent007_workflows::WorkflowRunRequest = store
        .read_json_artifact(session, "workflow-request.json")?;
    let state: agent007_workflows::WorkflowRunState = store
        .read_json_artifact(session, "workflow-state.json")?;
    let workflow_ref = store
        .read_json_artifact_optional::<agent007_workflows::WorkflowSourceRef>(
            session,
            "workflow-source.json",
        )?
        .map(|source| source.workflow_ref)
        .unwrap_or_else(|| request.workflow.clone());
    let def = load_workflow_def(&workflow_ref)?;
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

fn hosted_workflow_response(
    session: &str,
    request: &agent007_workflows::WorkflowRunRequest,
    progress: &agent007_workflows::HostedWorkflowProgress,
    state: &agent007_workflows::WorkflowRunState,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "session": session,
        "mode": "hosted-mcp",
        "request": request,
        "progress": progress,
        "workflow_state": state,
        "available_tools": [
            "agent007_workflow_status",
            "agent007_workflow_next",
            "agent007_workflow_submit_step",
            "agent007_workflow_approve",
        ],
    }))?)
}

fn workflow_hosted_start(name: &str, task: &str) -> Result<String> {
    let def = load_workflow_def(name)?;
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
    let engine = hosted_workflow_engine();

    match engine.dispatch(&def, &mut state) {
        Ok(progress) => {
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
        }
        Err(error) => {
            let summary = format!("hosted workflow start failed: {}", error);
            let _ = store.finish_run(&run.id, false, &summary);
            Err(anyhow::anyhow!(summary))
        }
    }
}

fn workflow_hosted_next(session: &str) -> Result<String> {
    let (store, request, def, mut state) = load_hosted_workflow_session(session)?;
    let engine = hosted_workflow_engine();

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
            sync_hosted_run_metadata(&store, session, &progress)?;
            hosted_workflow_response(session, &request, &progress, &state)
        }
        Err(error) => {
            let summary = format!("hosted workflow dispatch failed: {}", error);
            let _ = store.finish_run(session, false, &summary);
            Err(anyhow::anyhow!(summary))
        }
    }
}

fn workflow_hosted_submit_step(session: &str, step: &str, output: &str) -> Result<String> {
    let (store, request, def, mut state) = load_hosted_workflow_session(session)?;
    let engine = hosted_workflow_engine();

    match engine.submit_step_output(&def, &mut state, step, output) {
        Ok(progress) => {
            store.write_json_artifact(session, "workflow-state.json", &state)?;
            store.append_note(
                session,
                "workflow-hosted-submit-step",
                serde_json::json!({
                    "workflow": request.workflow,
                    "step": step,
                    "progress": &progress,
                }),
            )?;
            sync_hosted_run_metadata(&store, session, &progress)?;
            hosted_workflow_response(session, &request, &progress, &state)
        }
        Err(error) => {
            let summary = format!("hosted workflow step submission failed: {}", error);
            let _ = store.finish_run(session, false, &summary);
            Err(anyhow::anyhow!(summary))
        }
    }
}

fn workflow_hosted_status(session: &str) -> Result<String> {
    let (store, request, def, mut state) = load_hosted_workflow_session(session)?;
    let engine = hosted_workflow_engine();

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
}

async fn execute_workflow_session(
    config: &Config,
    kind: &str,
    def: agent007_workflows::WorkflowDef,
    task: String,
    resume_state: Option<agent007_workflows::WorkflowRunState>,
    workflow_ref: Option<String>,
) -> Result<String> {
    let (stack, run_id) = create_traced_stack(config, kind, &format!("{}: {}", def.name, task)).await?;
    let runner = match resume_state {
        Some(state) => stack
            .workflow_runner
            .resume_from(stack.run_store.clone(), run_id.clone(), state),
        None => stack
            .workflow_runner
            .for_run(stack.run_store.clone(), run_id.clone()),
    };
    if let Some(workflow_ref) = workflow_ref {
        stack.run_store.write_json_artifact(
            &run_id,
            "workflow-source.json",
            &agent007_workflows::WorkflowSourceRef { workflow_ref },
        )?;
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
        Err(error) => {
            match &error {
                agent007_workflows::WorkflowError::ApprovalRequired { id } => {
                    let summary = format!(
                        "Workflow '{}' is waiting for approval on step '{}'. Run ID: {}",
                        def.name, id, run_id,
                    );
                    let _ = stack.run_store.finish_run_with_status(
                        &run_id,
                        agent007_core::run_store::RunStatus::AwaitingApproval,
                        &summary,
                    );
                    Err(anyhow::anyhow!(summary))
                }
                _ => {
                    let _ = stack.run_store.finish_run(&run_id, false, error.to_string());
                    Err(anyhow::anyhow!("workflow run failed: {}", error))
                }
            }
        }
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
    use agent007_zones::{ZoneChecker, ZoneConfig, FileOp};
    use std::path::Path;

    let zone_config = ZoneConfig {
        forbidden:    config.zones.forbidden.clone(),
        readonly:     config.zones.readonly.clone(),
        sensitive:    config.zones.sensitive.clone(),
        unrestricted: config.zones.unrestricted.clone(),
    };

    let checker = ZoneChecker::new(&zone_config)
        .map_err(|e| anyhow::anyhow!("zone config error: {}", e))?;

    let file_op = match operation.to_lowercase().as_str() {
        "read"    => FileOp::Read,
        "write"   => FileOp::Write,
        "execute" => FileOp::Write, // map execute to write (most restrictive non-delete)
        other     => return Err(anyhow::anyhow!(
            "Unknown operation '{}'. Use 'read', 'write', or 'execute'.", other
        )),
    };

    let path = Path::new(path_str);
    let zone = checker.zone_for(path);

    match checker.check(path, file_op) {
        Ok(()) => Ok(format!(
            "ALLOWED: {} on '{}' (zone: {})",
            operation, path_str, zone.as_str()
        )),
        Err(violation) => Ok(format!(
            "DENIED: {} on '{}' (zone: {}): {}",
            operation, path_str, zone.as_str(), violation
        )),
    }
}

// ── task submit helper ────────────────────────────────────────────────────────

async fn task_submit(config: &Config, task: String, persona: Option<String>) -> Result<String> {
    let task_id = uuid_v4();
    let description = match persona {
        Some(ref p) => format!("[persona:{}] {}", p, task),
        None        => task.clone(),
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
                let _ = stack.run_store.finish_run(&run_id, false, error.to_string());
                Err(error.into())
            }
        }
    } else {
        let run_id = create_delegate_run("task-submit", &description)?;
        let output = format!(
            "Task accepted in hosted MCP mode. ID: {task_id}\n\
             run_id: {run_id}\n\
             Host instruction: execute the task directly and persist important results with agent007_memory_write. \
             After completing, call agent007_record_tokens with run_id={run_id}, actual tokens used, and your model name.\n\
             Task: {description}"
        );
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
    let skills_dir = agent007_home().join("skills");
    std::fs::create_dir_all(&skills_dir)
        .map_err(|e| anyhow::anyhow!("failed to create skills dir: {}", e))?;

    // Sanitise file name: replace spaces with underscores, keep alphanumeric + _-
    let filename = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>();
    let path = skills_dir.join(format!("{}.md", filename));

    let content = format!(
        "---\nname: {}\ntrigger: {}\ndescription: {}\nmodel: {}\n---\n{}\n",
        name, trigger, description, model, template
    );

    std::fs::write(&path, &content)
        .map_err(|e| anyhow::anyhow!("failed to write skill file: {}", e))?;

    Ok(path.display().to_string())
}

// ── config show helper ────────────────────────────────────────────────────────

fn config_show(config: &Config) -> String {
    // Serialise to TOML; fall back to debug repr on error
    toml::to_string_pretty(config).unwrap_or_else(|_| format!("{:?}", config))
}

async fn mcp_tools_list(config: &Config) -> Result<String> {
    let (stack, run_id) = create_traced_stack(
        config,
        "mcp-tools-list",
        "list downstream MCP tools",
    )
    .await?;
    let tools = stack.tool_executor.list_mcp_tools().await?;
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
    let output = serde_json::to_string_pretty(&payload)?;
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
    match stack.tool_executor.call_mcp_tool(agent_id, name, args).await {
        Ok(result) => {
            let output = serde_json::to_string_pretty(&result)?;
            let _ = stack.run_store.finish_run(&run_id, true, &output);
            Ok(output)
        }
        Err(error) => {
            let _ = stack.run_store.finish_run(&run_id, false, error.to_string());
            Err(error.into())
        }
    }
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
    let workflow_request = store.read_json_artifact_optional::<serde_json::Value>(id, "workflow-request.json")?;
    let workflow_source = store.read_json_artifact_optional::<serde_json::Value>(id, "workflow-source.json")?;
    let workflow_state = store.read_json_artifact_optional::<serde_json::Value>(id, "workflow-state.json")?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "run": detail,
        "workflow_request": workflow_request,
        "workflow_source": workflow_source,
        "workflow_state": workflow_state,
    }))?)
}

fn compact_output_tool(config: &Config, command: &str, output: &str, level: &str) -> Result<String> {
    let level = parse_compact_level(level)?;
    let run_id = create_recorded_utility_run(config, "compact-output", command)?;
    let store = load_run_store();
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
}

fn budget_estimate_tool(
    config: &Config,
    task: Option<String>,
    text: Option<String>,
    max_prompt_tokens: u64,
    reserve_tokens: u64,
    max_response_tokens: u64,
) -> Result<String> {
    let text = text
        .or_else(|| task.clone())
        .unwrap_or_default();
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
}

// ── health check helper ───────────────────────────────────────────────────────

fn health_check(config: &Config) -> String {
    let home = agent007_home();

    let memory_dir   = home.join("memory");
    let skills_dir   = home.join("skills");
    let personas_dir = home.join("personas");

    let memory_ok = memory_dir.exists();

    let skills_count = count_files_with_ext(&skills_dir, "md");
    let personas_count = {
        // Built-in count + user overrides
        let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
            .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
        use agent007_core::PersonaProvider;
        registry.list().len()
    };

    let zones_configured = !config.zones.forbidden.is_empty()
        || !config.zones.readonly.is_empty()
        || !config.zones.sensitive.is_empty()
        || !config.zones.unrestricted.is_empty();

    let available_providers = {
        let mut providers = Vec::new();
        if std::env::var("ANTHROPIC_API_KEY").map(|v| !v.is_empty()).unwrap_or(false) {
            providers.push("claude");
        }
        if std::env::var("OPENAI_API_KEY").map(|v| !v.is_empty()).unwrap_or(false) {
            providers.push("codex");
        }
        if config.models.ollama.is_some() {
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
    let selected_provider = selected_runtime_provider(config)
        .unwrap_or_else(|| "hosted-mcp".to_string());
    let selected_model = selected_runtime_model(config)
        .unwrap_or_else(|| "host-llm".to_string());
    let runtime_mode = runtime_mode_label(config);

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
         selected model:    {}\n",
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
    )
}

// ── workflow plan (hosted MCP mode) ────────────────────────────────────────

async fn workflow_plan(_config: &Config, name: &str, task: &str) -> Result<String> {
    let workflows_dir = agent007_home().join("workflows");

    let path = {
        let yaml_path = workflows_dir.join(format!("{}.yaml", name));
        let yml_path  = workflows_dir.join(format!("{}.yml", name));
        if yaml_path.exists() {
            yaml_path
        } else if yml_path.exists() {
            yml_path
        } else {
            return Err(anyhow::anyhow!(
                "Workflow '{}' not found in {}",
                name, workflows_dir.display()
            ));
        }
    };

    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("failed to read workflow: {}", e))?;
    let def: agent007_workflows::types::WorkflowDef = serde_yaml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("failed to parse workflow YAML: {}", e))?;

    let personas_dir = agent007_home().join("personas");
    let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
        .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());

    let mut steps = Vec::new();
    for step in &def.steps {
        let persona = {
            use agent007_core::PersonaProvider;
            registry.get(&step.agent)
        };

        let rendered_prompt = step.prompt.as_deref()
            .unwrap_or("")
            .replace("{{task}}", task);

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
    let personas_dir = agent007_home().join("personas");
    let registry = agent007_personas::PersonaRegistry::load(&personas_dir)
        .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());

    use agent007_core::PersonaProvider;
    let personas = registry.list();

    let archetypes: Vec<serde_json::Value> = personas.iter().map(|p| {
        serde_json::json!({
            "name": p.name,
            "description": p.description,
            "preferred_model": p.preferred_model,
            "allowed_tools": p.allowed_tools,
            "system_prompt": p.system_prompt,
        })
    }).collect();

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
    allowed_tools: &[String],
) -> Result<String> {
    let personas_dir = agent007_home().join("personas");
    std::fs::create_dir_all(&personas_dir)
        .map_err(|e| anyhow::anyhow!("failed to create personas dir: {}", e))?;

    let filename = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>()
        .to_lowercase();
    let path = personas_dir.join(format!("{filename}.toml"));

    let tools_str = allowed_tools.iter()
        .map(|t| format!("\"{}\"", t))
        .collect::<Vec<_>>()
        .join(", ");

    let content = format!(
        "name            = \"{}\"\n\
         description     = \"{}\"\n\
         preferred_model = \"{}\"\n\
         allowed_tools   = [{}]\n\
         \n\
         system_prompt   = \"\"\"\n\
         {}\n\
         \"\"\"\n",
        name,
        description.replace('"', "\\\""),
        preferred_model,
        tools_str,
        system_prompt,
    );

    std::fs::write(&path, &content)
        .map_err(|e| anyhow::anyhow!("failed to write persona file: {}", e))?;

    Ok(path.display().to_string())
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

fn count_files_with_ext(dir: &std::path::Path, ext: &str) -> usize {
    if !dir.exists() {
        return 0;
    }
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path().extension().and_then(|x| x.to_str()) == Some(ext)
                })
                .count()
        })
        .unwrap_or(0)
}

/// Entry point: start MCP stdio server + web dashboard (unless `--no-dashboard`).
pub async fn execute(config: Arc<Config>, dashboard_port: u16, no_dashboard: bool) -> Result<()> {
    let mut shared_dispatcher: Option<Arc<LocalDispatcher>> = None;
    let mut shared_learning: Option<Arc<LearningDispatcher>> = None;

    if !no_dashboard {
        // Guard: if another process already owns the dashboard port, skip starting a new
        // instance so Zed + Claude Code share one dashboard instead of each spawning their own.
        let already_running = if let Some(port) = read_dashboard_port() {
            if dashboard_port_is_live(port).await {
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
            let stack = super::run::build_stack(&config).await?;
            shared_dispatcher = Some(stack.dispatcher.clone());
            shared_learning = Some(stack.learning_dispatcher.clone());
            let collector = stack.feedback_collector.clone();
            stack.tracker.spawn(async move {
                if let Err(e) = collector.run().await {
                    tracing::warn!("feedback collector error: {e}");
                }
            });
        } else {
        // Start the dashboard inline regardless of whether stdin is a terminal.
        // The MCP stdio protocol and the HTTP web server use completely different
        // transports and coexist in the same process without conflict.
        let stack = super::run::build_stack(&config).await?;
        let standalone_mode = standalone_mode_available(&config);
        let runtime_mode = runtime_mode_label(&config).to_string();
        let provider_label = match (
            selected_runtime_provider(&config),
            selected_runtime_model(&config),
        ) {
            (Some(provider), Some(model)) if provider != model => format!("{provider} / {model}"),
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

        let web = agent007_web::WebServer::new(
            stack.dispatcher.clone(),
            stack.learning_dispatcher.clone(),
            stack.model_router.clone(),
            Some(stack.workflow_runner.clone()),
            stack.cancel.clone(),
            standalone_mode,
            runtime_mode,
            provider_label,
        );

        tokio::spawn(async move {
            let (actual_port, listener) = find_free_port_with_listener(dashboard_port).await;
            eprintln!("[agent007] web dashboard: http://localhost:{actual_port}");
            persist_dashboard_port(actual_port);
            if let Err(e) = web.run_with_listener(listener).await {
                eprintln!("[agent007] web dashboard error: {e}");
            }
        });

        tracing::info!(
            "agent007 MCP server starting (stdio) + web dashboard on port {dashboard_port}"
        );
        } // end else already_running
    } else {
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
async fn find_free_port_with_listener(preferred: u16) -> (u16, tokio::net::TcpListener) {
    for offset in 0u16..50 {
        let port = preferred.wrapping_add(offset);
        let addr = format!("0.0.0.0:{port}");
        if let Ok(listener) = tokio::net::TcpListener::bind(&addr).await {
            return (port, listener);
        }
    }
    // Last-ditch: let the OS assign any free port.
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0")
        .await
        .expect("could not bind to any port");
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(preferred);
    (port, listener)
}

/// Write the active dashboard port to `.agent007/memory/project/dashboard_port.md`
/// so other tools (TUI, scripts, health checks) can discover it.
fn persist_dashboard_port(port: u16) {
    let store = memory_store();
    let scoped = store.scoped("project");
    let _ = scoped.write("dashboard_port", &port.to_string());
    let _ = scoped.write("dashboard_url", &format!("http://localhost:{port}"));
}

#[allow(dead_code)]
async fn ensure_dashboard_sidecar(preferred_port: u16) -> Result<Option<u16>> {
    if let Some(port) = read_dashboard_port() {
        if dashboard_port_is_live(port).await {
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
    let path = agent007_home()
        .join("memory")
        .join("project")
        .join("dashboard_port.md");
    let raw = std::fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;

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
        assert!(names.contains(&"agent007_persona_list"), "missing agent007_persona_list");
        assert!(names.contains(&"agent007_persona_show"), "missing agent007_persona_show");
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
        assert!(parts[2].starts_with('4'), "UUID version nibble must be 4, got: {}", id);
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

        let completed = workflow_hosted_submit_step(&session, "research", "notes v1").unwrap();
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

        let waiting = workflow_hosted_submit_step(&session, "plan", "draft plan").unwrap();
        let waiting: serde_json::Value = serde_json::from_str(&waiting).unwrap();
        assert_eq!(waiting["progress"]["status"], "awaiting-approval");

        let approval = workflow_approve(
            &session,
            None,
            "edit",
            Some("approved plan".to_string()),
        )
        .unwrap();
        assert!(approval.contains("agent007_workflow_next"));

        let resumed = workflow_hosted_next(&session).unwrap();
        let resumed: serde_json::Value = serde_json::from_str(&resumed).unwrap();
        assert_eq!(resumed["progress"]["status"], "succeeded");
        assert_eq!(resumed["workflow_state"]["outputs"]["plan"], "approved plan");

        std::env::remove_var("AGENT007_HOME");
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
        assert!(detail.artifacts.contains(&"compact-output.json".to_string()));
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
        assert!(result.contains("ALLOWED"), "expected ALLOWED, got: {}", result);
    }

    #[test]
    fn zone_check_denied_path() {
        let mut config = Config::default();
        config.zones.forbidden = vec!["secrets/".to_string()];
        let result = zone_check(&config, "secrets/token", "read").unwrap();
        assert!(result.contains("DENIED"), "expected DENIED, got: {}", result);
    }
}
