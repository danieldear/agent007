use std::sync::Arc;

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    model::{
        CallToolResult, Content, Implementation, InitializeRequestParam, ListToolsResult,
        PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool,
    },
    serve_server,
    service::{RequestContext, RoleServer},
    transport::io::stdio,
};
use serde_json::Map;

use crate::config::Config;
use super::run::{agent007_home, build_stack};
use super::skill::{list_skills, run_skill, SkillSummary};

/// MCP server that exposes agent007 tools to Claude Code (or any MCP client).
pub struct Agent007Server {
    config: Arc<Config>,
}

impl Agent007Server {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    fn tool_defs() -> Vec<Tool> {
        vec![
            // ── existing 5 tools ───────────────────────────────────────────
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
                "List all available skills loaded from ~/.agent007/skills/",
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

            // 4. Workflow list
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
                            "description": "Model to use (default: claude-sonnet-4-6)"
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
        ]
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
            "Use agent007_run to run tasks, agent007_skill_list to browse skills, \
             and agent007_skill_run to execute a specific skill."
                .to_string(),
        );
        info
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<rmcp::model::InitializeResult, rmcp::model::ErrorData> {
        Ok(self.get_info().into())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, rmcp::model::ErrorData> {
        Ok(ListToolsResult::with_all_items(Self::tool_defs()))
    }

    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, rmcp::model::ErrorData> {
        match request.name.as_ref() {
            // ── existing 5 ────────────────────────────────────────────────
            "agent007_run" => {
                let task = extract_string(request.arguments.as_ref(), "task")?;
                match run_task(&self.config, task).await {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }
            "agent007_skill_list" => {
                let skills_dir = agent007_home().join("skills");
                match list_skills(&skills_dir).await {
                    Ok(skills) => {
                        Ok(CallToolResult::success(vec![Content::text(format_skills(&skills))]))
                    }
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }
            "agent007_skill_run" => {
                let trigger = extract_string(request.arguments.as_ref(), "trigger")?;
                let args = string_or_default(request.arguments.as_ref(), "args", "");
                match run_skill_mcp(&self.config, trigger, args).await {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
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

            // 4. Workflow list
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
                match workflow_run(&self.config, &name, &task).await {
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
                let task    = extract_string(request.arguments.as_ref(), "task")?;
                let persona = string_or_default(request.arguments.as_ref(), "persona", "");
                match task_submit(&self.config, task, if persona.is_empty() { None } else { Some(persona) }).await {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
                }
            }

            // 13. Skill create
            "agent007_skill_create" => {
                let name        = extract_string(request.arguments.as_ref(), "name")?;
                let trigger     = extract_string(request.arguments.as_ref(), "trigger")?;
                let description = extract_string(request.arguments.as_ref(), "description")?;
                let template    = extract_string(request.arguments.as_ref(), "template")?;
                let model       = string_or_default(request.arguments.as_ref(), "model", "claude-sonnet-4-6");
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

async fn run_task(config: &Config, task: String) -> Result<String> {
    // Use dry-run mode so there is no TUI when called from MCP
    std::env::set_var("AGENT007_DRY_RUN", "1");
    let stack = build_stack(config).await?;
    let core_task = agent007_core::Task::new(&task);
    stack.orchestrator.run(core_task).await?;
    Ok("Task submitted to agent007 orchestrator.".to_string())
}

async fn run_skill_mcp(config: &Config, trigger: String, args: String) -> Result<String> {
    let stack = build_stack(config).await?;
    run_skill(&trigger, &args, &stack.skill_executor).await
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
    let home = agent007_home();
    let scope_dir = if scope.is_empty() || scope == "global" {
        home.join("memory")
    } else {
        home.join("memory").join(scope)
    };

    if !scope_dir.exists() {
        return Ok(vec![]);
    }

    let mut keys = Vec::new();
    for entry in std::fs::read_dir(&scope_dir)
        .map_err(|e| anyhow::anyhow!("cannot read memory dir: {}", e))?
    {
        let entry = entry.map_err(|e| anyhow::anyhow!("{}", e))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                keys.push(stem.to_string());
            }
        }
    }
    keys.sort();
    Ok(keys)
}

// ── workflow helpers ──────────────────────────────────────────────────────────

fn workflow_list() -> Result<Vec<String>> {
    let workflows_dir = agent007_home().join("workflows");
    if !workflows_dir.exists() {
        return Ok(vec![]);
    }

    let mut names = Vec::new();
    for entry in std::fs::read_dir(&workflows_dir)
        .map_err(|e| anyhow::anyhow!("cannot read workflows dir: {}", e))?
    {
        let entry = entry.map_err(|e| anyhow::anyhow!("{}", e))?;
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        if ext == Some("yaml") || ext == Some("yml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

async fn workflow_run(config: &Config, name: &str, task: &str) -> Result<String> {
    let workflows_dir = agent007_home().join("workflows");

    // Try .yaml first, then .yml
    let path = {
        let yaml_path = workflows_dir.join(format!("{}.yaml", name));
        let yml_path  = workflows_dir.join(format!("{}.yml", name));
        if yaml_path.exists() {
            yaml_path
        } else if yml_path.exists() {
            yml_path
        } else {
            return Err(anyhow::anyhow!(
                "Workflow '{}' not found in {} — run 'agent007 workflow list' to see available workflows",
                name,
                workflows_dir.display()
            ));
        }
    };

    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("failed to read workflow file: {}", e))?;

    let def: agent007_workflows::types::WorkflowDef = serde_yaml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("failed to parse workflow YAML: {}", e))?;

    // Build the stack without forcing dry-run — use real providers when available.
    let stack = build_stack(config).await?;
    let result = stack.workflow_runner.run(&def, task).await
        .map_err(|e| anyhow::anyhow!("workflow run failed: {}", e))?;

    // Format outputs as a readable report
    let mut report = format!(
        "# Workflow: {}\nTask: {}\nSteps completed: {}/{}\n\n",
        def.name, task, result.steps_completed, result.steps_total
    );
    for (key, value) in &result.outputs {
        report.push_str(&format!("## {}\n{}\n\n", key, value));
    }
    Ok(report)
}

// ── git helpers ───────────────────────────────────────────────────────────────

fn git_run(args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
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
    std::env::set_var("AGENT007_DRY_RUN", "1");
    let stack = build_stack(config).await?;

    let task_id = uuid_v4();
    let description = match persona {
        Some(ref p) => format!("[persona:{}] {}", p, task),
        None        => task.clone(),
    };

    let core_task = agent007_core::Task::new(&description);
    stack.orchestrator.run(core_task).await?;

    Ok(format!("Task submitted. ID: {}", task_id))
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

    format!(
        "agent007 health\n\
         ───────────────\n\
         home:              {}\n\
         memory dir:        {} ({})\n\
         skills:            {} loaded\n\
         personas:          {} available\n\
         zones configured:  {}\n",
        home.display(),
        memory_dir.display(),
        if memory_ok { "exists" } else { "missing" },
        skills_count,
        personas_count,
        if zones_configured { "yes" } else { "no" },
    )
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

/// Entry point: start the MCP stdio server.
pub async fn execute(config: Arc<Config>) -> Result<()> {
    tracing::info!("agent007 MCP server starting (stdio transport)");
    let server = Agent007Server::new(config);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_defs_contains_persona_list_and_show() {
        let defs = Agent007Server::tool_defs();
        let names: Vec<&str> = defs.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"agent007_persona_list"), "missing agent007_persona_list");
        assert!(names.contains(&"agent007_persona_show"), "missing agent007_persona_show");
    }

    #[test]
    fn tool_defs_has_at_least_20_tools() {
        let defs = Agent007Server::tool_defs();
        assert!(
            defs.len() >= 20,
            "expected at least 20 tools, got {}",
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
            "agent007_workflow_list",
            "agent007_workflow_run",
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
        ];
        for tool_name in &expected {
            assert!(names.contains(tool_name), "missing tool: {}", tool_name);
        }
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
        // Use a temp AGENT007_HOME so we don't touch the real one
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        let keys = memory_list("nonexistent_scope").unwrap();
        assert!(keys.is_empty());
        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn workflow_list_on_missing_dir_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT007_HOME", tmp.path());
        let names = workflow_list().unwrap();
        assert!(names.is_empty());
        std::env::remove_var("AGENT007_HOME");
    }

    #[test]
    fn skill_create_writes_file() {
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
