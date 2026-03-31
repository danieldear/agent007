use std::path::{Path, PathBuf};
use anyhow::Result;

use crate::config::Config;
use super::run::{selected_runtime_model, selected_runtime_provider, standalone_mode_available};

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn ok(msg: &str) {
    println!("  {GREEN}✓{RESET} {msg}");
}
fn warn(msg: &str) {
    println!("  {YELLOW}⚠{RESET} {msg}");
}
fn info(msg: &str) {
    println!("  {CYAN}→{RESET} {msg}");
}
fn section(msg: &str) {
    println!("\n{BOLD}{msg}{RESET}");
}

fn ensure_dir(path: &Path, label: &str) -> Result<bool> {
    if path.exists() {
        ok(&format!("{label} already exists"));
        Ok(false)
    } else {
        std::fs::create_dir_all(path)?;
        ok(&format!("{label} created"));
        Ok(true)
    }
}

fn write_if_missing(path: &Path, content: &str, label: &str) -> Result<bool> {
    if path.exists() {
        ok(&format!("{label} already exists — skipped"));
        Ok(false)
    } else {
        std::fs::write(path, content)?;
        ok(&format!("{label} written"));
        Ok(true)
    }
}

pub async fn execute(
    config: std::sync::Arc<Config>,
    force: bool,
    global: bool,
    do_claude: bool,
    do_cursor: bool,
    do_codex: bool,
    do_zed: bool,
) -> Result<()> {
    let home = if global {
        super::run::agent007_global_home()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| dirs_home())
            .join(".agent007")
    };

    let project_dir = std::env::current_dir().unwrap_or_else(|_| dirs_home());
    let claude_scope_dir = if global {
        dirs_home().join(".claude")
    } else {
        project_dir.join(".claude")
    };
    let cursor_scope_dir = project_dir.join(".cursor");
    let codex_scope_dir = if global {
        dirs_home().join(".codex")
    } else {
        project_dir.join(".codex")
    };
    let scope_label = if global { "~/.agent007/ (global)" } else { ".agent007/ (project-local)" };
    let mut ide_targets = Vec::new();
    if do_claude {
        ide_targets.push("Claude Code");
    }
    if do_cursor {
        ide_targets.push("Cursor");
    }
    if do_codex {
        ide_targets.push("Codex");
    }
    if do_zed {
        ide_targets.push("Zed");
    }
    let ide_label = if ide_targets.is_empty() {
        "none (--no-ide)".to_string()
    } else {
        ide_targets.join(" + ")
    };

    println!();
    println!("{BOLD}{CYAN}agent007{RESET} — initializing your workspace");
    println!("{DIM}home: {}{RESET}", home.display());
    println!("{DIM}scope: {scope_label}{RESET}");
    println!("{DIM}IDE:   {ide_label}{RESET}");

    // ── 1. Directory structure ──────────────────────────────────────────────
    section("1. Creating directory structure");
    ensure_dir(&home, ".agent007/")?;
    ensure_dir(&home.join("skills"), "skills/")?;
    ensure_dir(&home.join("personas"), "personas/")?;
    ensure_dir(&home.join("workflows"), "workflows/")?;
    ensure_dir(&home.join("memory"), "memory/")?;
    ensure_dir(&home.join("hooks"), "hooks/")?;
    ensure_dir(&home.join("audit"), "audit/")?;
    ensure_dir(&home.join("vectordb"), "vectordb/")?;
    ensure_dir(&home.join("checkpoints"), "checkpoints/")?;
    ensure_dir(&home.join("sessions"), "sessions/")?;

    // ── 2. Default config ───────────────────────────────────────────────────
    section("2. Writing default configuration");
    let config_path = home.join("config.toml");
    write_if_missing(&config_path, DEFAULT_CONFIG, "config.toml")?;

    // ── 3. Default hooks ────────────────────────────────────────────────────
    section("3. Writing default hooks");
    let hooks_path = home.join("hooks").join("hooks.toml");
    write_if_missing(&hooks_path, DEFAULT_HOOKS, "hooks/hooks.toml")?;

    // ── 3b. Seed built-in skills ─────────────────────────────────────────────
    section("3b. Seeding built-in skills");
    let skills_dir_seed = home.join("skills");
    let mut built_in_skill_count = 0usize;
    for (filename, content) in crate::built_in_skills::ALL_SKILLS {
        if write_if_missing(&skills_dir_seed.join(filename), content, &format!("skills/{filename}"))? {
            built_in_skill_count += 1;
        }
    }
    if built_in_skill_count > 0 {
        ok(&format!("{built_in_skill_count} built-in skills seeded"));
    }

    // ── 4. Built-in workflows ───────────────────────────────────────────────
    section("4. Writing built-in workflows");
    let wf_dir = home.join("workflows");
    write_if_missing(&wf_dir.join("log-analysis.yaml"),  WORKFLOW_LOG_ANALYSIS,  "workflows/log-analysis.yaml")?;
    write_if_missing(&wf_dir.join("code-review.yaml"),   WORKFLOW_CODE_REVIEW,   "workflows/code-review.yaml")?;
    write_if_missing(&wf_dir.join("sparc.yaml"),         WORKFLOW_SPARC,         "workflows/sparc.yaml")?;
    write_if_missing(&wf_dir.join("tdd.yaml"),           WORKFLOW_TDD,           "workflows/tdd.yaml")?;

    // ── 5. Seed ALL built-in personas as editable TOML files ────────────────
    section("5. Seeding built-in personas");
    let personas_dir = home.join("personas");
    let registry = agent007_personas::PersonaRegistry::built_in();
    let personas = {
        use agent007_core::PersonaProvider;
        registry.list()
    };
    let mut persona_count = 0usize;
    for spec in &personas {
        let filename = spec.name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect::<String>()
            .to_lowercase();
        let path = personas_dir.join(format!("{filename}.toml"));
        let tools_str = spec.allowed_tools.iter()
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
            spec.name,
            spec.description.replace('"', "\\\""),
            spec.preferred_model,
            tools_str,
            spec.system_prompt,
        );
        if write_if_missing(&path, &content, &format!("personas/{filename}.toml"))? {
            persona_count += 1;
        }
    }
    if persona_count > 0 {
        ok(&format!("{persona_count} persona files seeded"));
    }

    // ── 6. IDE integrations ─────────────────────────────────────────────────
    let mut step = 6;

    if do_claude {
        section(&format!("{step}. Registering MCP server with Claude Code"));
        register_mcp_in_settings("agent007", &claude_scope_dir, force)?;
        step += 1;

        section(&format!("{step}. Installing slash commands for Claude Code"));
        let commands_dir = claude_scope_dir.join("commands");
        if !commands_dir.exists() {
            std::fs::create_dir_all(&commands_dir)?;
            ok("commands/ created");
        }

        let skills_dir = home.join("skills");
        let mut installed = 0usize;
        if let Ok(entries) = std::fs::read_dir(&skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let stem = path.file_stem().unwrap().to_string_lossy();
                let cmd_file = commands_dir.join(format!("agent007-{stem}.md"));
                if !cmd_file.exists() || force {
                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                    let description = content
                        .lines()
                        .find(|l| l.starts_with("description:"))
                        .map(|l| l.trim_start_matches("description:").trim().to_string())
                        .unwrap_or_else(|| format!("Run /{stem} skill"));
                    let trigger = content
                        .lines()
                        .find(|l| l.starts_with("trigger:"))
                        .map(|l| l.trim_start_matches("trigger:").trim().to_string())
                        .unwrap_or_else(|| format!("/{stem}"));
                    let cmd_content = format!(
                        "{description}\n\nUse the mcp__agent007__agent007_skill_run tool with trigger \"{trigger}\" and args \"$ARGUMENTS\".\n"
                    );
                    std::fs::write(&cmd_file, cmd_content)?;
                    installed += 1;
                }
            }
        }
        if installed > 0 {
            ok(&format!("{installed} slash commands installed"));
        } else {
            ok("All slash commands already installed");
        }
        step += 1;

        section(&format!("{step}. Installing Claude Code sub-agents"));
        let agents_dir = claude_scope_dir.join("agents");
        if !agents_dir.exists() {
            std::fs::create_dir_all(&agents_dir)?;
            ok("agents/ created");
        }
        write_if_missing(&agents_dir.join("agent007-architect.md"), CLAUDE_AGENT_ARCHITECT, "agents/agent007-architect.md")?;
        write_if_missing(&agents_dir.join("agent007-analyst.md"),   CLAUDE_AGENT_ANALYST,   "agents/agent007-analyst.md")?;
        step += 1;
    }

    if do_cursor {
        section(&format!("{step}. Registering MCP server with Cursor"));
        register_cursor_mcp(&cursor_scope_dir, force)?;
        step += 1;
    }

    if do_codex {
        section(&format!("{step}. Registering MCP server with Codex"));
        register_codex_mcp(&codex_scope_dir, force)?;
        step += 1;
    }

    if do_zed {
        section(&format!("{step}. Registering LSP server with Zed"));
        let zed_scope_dir = if global {
            dirs_home().join(".config").join("zed")
        } else {
            project_dir.join(".zed")
        };
        register_zed(&zed_scope_dir, &project_dir, force)?;
        step += 1;
    }

    let _ = step;

    // ── 9. Environment check ───────────────────────────────────────────────
    section("9. Environment check");

    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let openai_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    if anthropic_key.is_empty() {
        info("ANTHROPIC_API_KEY not set");
    } else {
        ok(&format!("ANTHROPIC_API_KEY set ({} chars)", anthropic_key.len()));
    }
    if openai_key.is_empty() {
        info("OPENAI_API_KEY not set");
    } else {
        ok(&format!("OPENAI_API_KEY set ({} chars)", openai_key.len()));
    }
    if standalone_mode_available(&config) {
        let provider = selected_runtime_provider(&config)
            .unwrap_or_else(|| "unknown".to_string());
        let model = selected_runtime_model(&config)
            .unwrap_or_else(|| "unknown".to_string());
        ok(&format!("standalone mode available via {provider} ({model})"));
    } else {
        info("No standalone provider configured — hosted MCP mode active (host LLM handles reasoning via MCP)");
        info("Set OPENAI_API_KEY, ANTHROPIC_API_KEY, or [models.ollama] for standalone mode");
    }

    let git_ok = std::process::Command::new("git").arg("--version").output().is_ok();
    if git_ok { ok("git available") } else { warn("git not found in PATH") }

    // ── 10. Summary ────────────────────────────────────────────────────────
    let skill_count = std::fs::read_dir(&home.join("skills"))
        .map(|d| d.flatten().filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md")).count())
        .unwrap_or(0);

    println!();
    println!("{BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}");
    println!("{BOLD}{GREEN}agent007 is ready!{RESET}");
    println!("{BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}");
    println!();
    println!("  Home:        {DIM}{}{RESET}", home.display());
    println!("  Personas:    {GREEN}{}{RESET} available", personas.len());
    println!("  Skills:      {GREEN}{skill_count}{RESET} loaded");
    println!("  Workflows:   {GREEN}4{RESET} built-in (log-analysis, code-review, sparc, tdd)");
    println!("  MCP server:  {GREEN}agent007 serve{RESET}");
    println!("  Dashboard:   {CYAN}http://localhost:8007{RESET} (auto-starts with serve)");
    println!("  IDE:         {GREEN}{ide_label}{RESET}");
    println!();
    if !standalone_mode_available(&config) {
        println!("{DIM}Mode: hosted-mcp (host LLM handles reasoning via MCP){RESET}");
    } else {
        println!("{DIM}Mode: standalone (agent007 makes its own API calls){RESET}");
    }
    println!();
    println!("{DIM}Quick start (from Claude Code / Cursor / Codex):{RESET}");
    println!("  Use agent007 tools via MCP for hosted mode");
    println!("  Or configure Ollama / OPENAI_API_KEY / ANTHROPIC_API_KEY for standalone execution");
    println!("  Codex uses mcp_servers.agent007 from .codex/config.toml");
    println!();

    Ok(())
}

/// Write (or merge) the agent007 MCP server entry into <claude_dir>/settings.json.
/// Uses `cmd` as the command name (typically `"agent007"` from PATH).
fn register_mcp_in_settings(cmd: &str, claude_dir: &Path, force: bool) -> Result<()> {
    let settings_path = claude_dir.join("settings.json");

    let mut root: serde_json::Value = if settings_path.exists() {
        let raw = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&raw).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if servers.contains_key("agent007") && !force {
        ok(&format!("agent007 already registered in {}", settings_path.display()));
        return Ok(());
    }

    let entry = serde_json::json!({
        "command": cmd,
        "args": ["serve"]
    });
    servers.insert("agent007".to_string(), entry);

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&settings_path, serde_json::to_string_pretty(&root)?)?;
    ok(&format!("Wrote mcpServers.agent007 → {cmd}"));
    ok(&format!("  config: {}", settings_path.display()));
    println!();
    warn("Restart Claude Code to activate the MCP server");
    info("New Claude Code windows will pick it up automatically");
    Ok(())
}

/// Write the agent007 MCP server entry into <cursor_dir>/mcp.json.
fn register_cursor_mcp(cursor_dir: &Path, force: bool) -> Result<()> {
    let mcp_path = cursor_dir.join("mcp.json");

    let mut root: serde_json::Value = if mcp_path.exists() {
        let raw = std::fs::read_to_string(&mcp_path)?;
        serde_json::from_str(&raw).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if servers.contains_key("agent007") && !force {
        ok(&format!("agent007 already registered in {}", mcp_path.display()));
        return Ok(());
    }

    let entry = serde_json::json!({
        "command": "agent007",
        "args": ["serve"]
    });
    servers.insert("agent007".to_string(), entry);

    if let Some(parent) = mcp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&root)?)?;
    ok(&format!("Wrote mcpServers.agent007 → agent007"));
    ok(&format!("  config: {}", mcp_path.display()));
    println!();
    info("Restart Cursor to activate the MCP server");
    Ok(())
}

/// Write the agent007 MCP server entry into <codex_dir>/config.toml.
fn register_codex_mcp(codex_dir: &Path, force: bool) -> Result<()> {
    let config_path = codex_dir.join("config.toml");

    let mut root: toml::Value = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)?;
        toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let servers = root
        .as_table_mut()
        .unwrap()
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .unwrap();

    if servers.contains_key("agent007") && !force {
        ok(&format!("agent007 already registered in {}", config_path.display()));
        return Ok(());
    }

    let mut entry = toml::map::Map::new();
    entry.insert("command".to_string(), toml::Value::String("agent007".to_string()));
    entry.insert(
        "args".to_string(),
        toml::Value::Array(vec![
            toml::Value::String("serve".to_string()),
            toml::Value::String("--no-dashboard".to_string()),
        ]),
    );
    servers.insert("agent007".to_string(), toml::Value::Table(entry));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, toml::to_string_pretty(&root)?)?;
    ok("Wrote mcp_servers.agent007 → agent007");
    ok(&format!("  config: {}", config_path.display()));
    println!();
    warn("Restart Codex to activate the MCP server");
    info("Codex uses `serve --no-dashboard` by default for stdio MCP mode");
    Ok(())
}

/// Resolve the absolute path of the installed agent007 binary.
/// Falls back to the current executable path, then bare "agent007".
fn which_agent007() -> String {
    // Try `which agent007` first
    if let Ok(out) = std::process::Command::new("which").arg("agent007").output() {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }
    // Fall back to the running executable
    if let Ok(exe) = std::env::current_exe() {
        return exe.display().to_string();
    }
    "agent007".to_string()
}

/// Write Zed integration into <zed_dir>/:
///   settings.json — LSP binary + MCP context_server + agent tool permissions
///   tasks.json    — agent007 command palette tasks
///   AGENTS.md     — rules file wiring Zed AI to agent007 workflows (project root)
/// Project-local: .zed/ — Global: ~/.config/zed/
fn register_zed(zed_dir: &Path, project_dir: &Path, force: bool) -> Result<()> {
    if let Some(parent) = zed_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(zed_dir)?;

    register_zed_settings(zed_dir, force)?;
    register_zed_tasks(zed_dir, force)?;
    register_zed_rules(project_dir, force)?;
    Ok(())
}

fn register_zed_settings(zed_dir: &Path, force: bool) -> Result<()> {
    let settings_path = zed_dir.join("settings.json");

    let mut root: serde_json::Value = if settings_path.exists() {
        let raw = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&raw).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let binary_path = which_agent007();
    let obj = root.as_object_mut().unwrap();

    // ── LSP ────────────────────────────────────────────────────────────────
    let lsp = obj
        .entry("lsp")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if lsp.contains_key("agent007") && !force {
        ok(&format!("LSP already registered in {}", settings_path.display()));
    } else {
        lsp.insert("agent007".to_string(), serde_json::json!({
            "binary": {
                "path": binary_path,
                "arguments": ["serve-lsp", "--stdio"]
            }
        }));
        ok(&format!("Wrote lsp.agent007 → {binary_path} serve-lsp --stdio"));
    }

    // ── MCP context_server ─────────────────────────────────────────────────
    let ctx = obj
        .entry("context_servers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if ctx.contains_key("agent007") && !force {
        ok(&format!("MCP context_server already registered in {}", settings_path.display()));
    } else {
        ctx.insert("agent007".to_string(), serde_json::json!({
            "command": binary_path,
            "args": ["serve", "--no-dashboard"],
            "env": {}
        }));
        ok(&format!("Wrote context_servers.agent007 → {binary_path} serve --no-dashboard"));
    }

    // ── Agent tool permissions ─────────────────────────────────────────────
    // Auto-allow all agent007 MCP tools so Zed doesn't prompt on every call.
    let agent = obj
        .entry("agent")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    let perms = agent
        .entry("tool_permissions")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    let tools = perms
        .entry("tools")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if tools.contains_key("mcp:agent007") && !force {
        ok("agent tool permissions already set");
    } else {
        tools.insert("mcp:agent007".to_string(), serde_json::json!({
            "default": "allow"
        }));
        ok("Wrote agent.tool_permissions.mcp:agent007 → allow");
    }

    std::fs::write(&settings_path, serde_json::to_string_pretty(&root)?)?;
    ok(&format!("  config: {}", settings_path.display()));
    println!();
    warn("Restart Zed to activate the LSP and MCP server");
    Ok(())
}

fn register_zed_rules(project_dir: &Path, force: bool) -> Result<()> {
    // Zed checks AGENTS.md before CLAUDE.md, so this takes precedence.
    let rules_path = project_dir.join("AGENTS.md");
    if rules_path.exists() && !force {
        ok(&format!("AGENTS.md already exists — skipped ({})", rules_path.display()));
        return Ok(());
    }
    std::fs::write(&rules_path, ZED_AGENTS_MD)?;
    ok(&format!("Wrote AGENTS.md → {}", rules_path.display()));
    info("Zed auto-loads AGENTS.md into every Agent Panel interaction");
    Ok(())
}

fn register_zed_tasks(zed_dir: &Path, force: bool) -> Result<()> {
    let tasks_path = zed_dir.join("tasks.json");

    if tasks_path.exists() && !force {
        ok(&format!("tasks.json already exists — skipped ({})", tasks_path.display()));
        return Ok(());
    }

    let binary_path = which_agent007();
    let tasks = serde_json::json!([
        {
            "label": "agent007: run task",
            "command": binary_path,
            "args": ["run", "$ZED_SELECTED_TEXT"],
            "reveal": "always",
            "hide": "on_success",
            "save": "all"
        },
        {
            "label": "agent007: skill list",
            "command": binary_path,
            "args": ["skill", "list"],
            "reveal": "always",
            "hide": "never"
        },
        {
            "label": "agent007: serve (MCP + dashboard)",
            "command": binary_path,
            "args": ["serve"],
            "reveal": "no_focus",
            "hide": "never",
            "allow_concurrent_runs": false
        },
        {
            "label": "agent007: dashboard",
            "command": binary_path,
            "args": ["dashboard"],
            "reveal": "no_focus",
            "hide": "on_success"
        },
        {
            "label": "agent007: build frontend",
            "command": "npm",
            "args": ["run", "build"],
            "cwd": "$ZED_WORKTREE_ROOT/crates/web/frontend",
            "reveal": "always",
            "hide": "on_success"
        }
    ]);

    std::fs::write(&tasks_path, serde_json::to_string_pretty(&tasks)?)?;
    ok(&format!("Wrote tasks.json ({} tasks)", 5));
    ok(&format!("  config: {}", tasks_path.display()));
    info("Run tasks via: Zed command palette → 'task: spawn'");
    Ok(())
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

// ── Default file contents ───────────────────────────────────────────────────

const DEFAULT_CONFIG: &str = r#"# agent007 configuration
# Full docs: https://github.com/neo/agent007

[core]
max_agents = 8
task_queue_capacity = 256

[models]
default = "codex"

[models.routing]
code_completion = "codex"
reasoning = "codex"
sensitive = "claude"
fast_local = "ollama"

[models.codex]
default_model = "gpt-5.3-codex"

[models.claude]
default_model = "claude-sonnet-4-6"

# Uncomment to enable local standalone execution via Ollama.
# [models.ollama]
# base_url = "http://localhost:11434"
# default_model = "llama3"

[learning]
[learning.reward_weights]
completion  = 1.0
user_rating = 2.0
tool_errors = -0.5
retries     = -0.25

[zones]
# Paths agent007 must never read or write (glob patterns)
forbidden    = ["**/.env", "**/*.pem", "**/*.key", "**/secrets/**"]
# Paths that are read-only
readonly     = ["**/Cargo.lock", "**/package-lock.json"]
# Paths that require extra confirmation before writing
sensitive    = ["**/src/auth/**", "**/migrations/**"]
# Paths with no restrictions
unrestricted = ["**/target/**", "**/node_modules/**"]
"#;

const DEFAULT_HOOKS: &str = r#"# agent007 hooks configuration
# Hooks run shell commands at key points in the agent lifecycle.

[[hooks]]
event    = "task_start"
command  = "echo 'agent007: task started'"
enabled  = false

[[hooks]]
event    = "task_complete"
command  = "echo 'agent007: task complete'"
enabled  = false

[[hooks]]
event    = "tool_blocked"
command  = "echo 'agent007: tool blocked by zone policy'"
enabled  = false
"#;

#[allow(dead_code)]
const EXAMPLE_WORKFLOW: &str = r#"# Example multi-agent workflow
name: code-review-workflow
description: Full code review — research, review, test suggestions

steps:
  - name: understand
    persona: Researcher
    task: "Understand the codebase structure and recent changes: {{ input }}"
    depends_on: []

  - name: review
    persona: CodeReviewer
    task: "Review the code for quality, security, and correctness: {{ input }}"
    depends_on: [understand]

  - name: tests
    persona: TestEngineer
    task: "Suggest missing tests based on the review findings: {{ input }}"
    depends_on: [review]
"#;

// ── Built-in workflows ──────────────────────────────────────────────────────

const WORKFLOW_LOG_ANALYSIS: &str = r#"name: log-analysis
description: >
  Parallel log analysis team. Specialists run concurrently; the synthesizer
  aggregates all findings into a final report.

steps:
  - id: find-errors
    agent: Researcher
    prompt: |
      You are a log error specialist. Analyze the following logs and extract ALL
      errors, exceptions, stack traces, and warnings. For each one provide:
      - Error type and message
      - Frequency / first/last occurrence
      - Likely root cause hypothesis

      Task / logs: {{task}}
    output: errors_report
    depends_on: []

  - id: find-patterns
    agent: Researcher
    prompt: |
      You are a log pattern analyst. Analyze the following logs and identify:
      - Recurring patterns and sequences
      - Anomalous spikes or gaps
      - Performance degradation signals
      - Correlation between events

      Task / logs: {{task}}
    output: patterns_report
    depends_on: []

  - id: security-check
    agent: Researcher
    prompt: |
      You are a security log analyst. Scan the following logs for:
      - Authentication failures or brute-force patterns
      - Unauthorized access attempts
      - Data exfiltration signals
      - Suspicious IP addresses or user agents

      Task / logs: {{task}}
    output: security_report
    depends_on: []

  - id: synthesize
    agent: Researcher
    prompt: |
      You are the lead analyst. Synthesize the specialist reports below into a
      single executive report with:
      1. Summary (3-5 bullet points)
      2. Critical issues (ranked by severity)
      3. Recommended fixes (ranked by priority)
      4. Next steps

      ERROR ANALYSIS:
      {{errors_report}}

      PATTERN ANALYSIS:
      {{patterns_report}}

      SECURITY ANALYSIS:
      {{security_report}}
    output: final_report
    depends_on: [find-errors, find-patterns, security-check]
"#;

const WORKFLOW_CODE_REVIEW: &str = r#"name: code-review
description: >
  Parallel code review team. Security, performance, and style reviewers run
  concurrently; the lead synthesizes findings.

steps:
  - id: security-review
    agent: Researcher
    prompt: |
      You are a security code reviewer. Review the following code for:
      - Injection vulnerabilities (SQL, command, XSS)
      - Authentication/authorization flaws
      - Insecure data handling (secrets, PII)
      - Dependency vulnerabilities

      Code / task: {{task}}
    output: security_findings
    depends_on: []

  - id: performance-review
    agent: Researcher
    prompt: |
      You are a performance code reviewer. Review the following code for:
      - Algorithmic complexity issues (O(n²), N+1 queries)
      - Memory leaks or excessive allocations
      - Blocking calls in async contexts
      - Missing indexes or caching opportunities

      Code / task: {{task}}
    output: performance_findings
    depends_on: []

  - id: quality-review
    agent: Researcher
    prompt: |
      You are a code quality reviewer. Review for:
      - Code smells and anti-patterns
      - Missing error handling
      - Test coverage gaps
      - Readability and maintainability issues

      Code / task: {{task}}
    output: quality_findings
    depends_on: []

  - id: synthesize
    agent: Researcher
    prompt: |
      Synthesize the three specialist reviews into a final code review report:

      SECURITY:
      {{security_findings}}

      PERFORMANCE:
      {{performance_findings}}

      QUALITY:
      {{quality_findings}}

      Format: severity-ranked issue list with line references and actionable fixes.
    output: review_report
    depends_on: [security-review, performance-review, quality-review]
"#;

const WORKFLOW_SPARC: &str = r#"name: sparc
description: >
  SPARC methodology pipeline: Spec → Pseudocode → Architecture → Refinement → Completion.
  Each phase feeds into the next.

steps:
  - id: spec
    agent: Researcher
    prompt: |
      SPARC Phase 1 — Specification.
      Write a detailed specification for: {{task}}
      Include: goals, constraints, user stories, acceptance criteria, edge cases.
    output: specification
    depends_on: []

  - id: pseudocode
    agent: Researcher
    prompt: |
      SPARC Phase 2 — Pseudocode.
      Based on this specification:
      {{specification}}

      Write structured pseudocode with logic flow, data structures, and algorithms.
    output: pseudocode
    depends_on: [spec]

  - id: architecture
    agent: Researcher
    prompt: |
      SPARC Phase 3 — Architecture.
      Spec: {{specification}}
      Pseudocode: {{pseudocode}}

      Design the system architecture: components, interfaces, data flow, dependencies.
    output: architecture
    depends_on: [pseudocode]

  - id: refinement
    agent: Researcher
    prompt: |
      SPARC Phase 4 — Refinement.
      Review the architecture for correctness, security, performance, and scalability.
      Architecture: {{architecture}}
      Identify and fix issues.
    output: refined_design
    depends_on: [architecture]

  - id: completion
    agent: Researcher
    prompt: |
      SPARC Phase 5 — Completion.
      Produce the final deliverable based on:
      Spec: {{specification}}
      Design: {{refined_design}}

      Include: implementation plan, test strategy, deployment notes, docs outline.
    output: final_deliverable
    depends_on: [refinement]
"#;

const WORKFLOW_TDD: &str = r#"name: tdd
description: >
  TDD pipeline: Red (write failing test) → Green (minimal implementation) → Blue (refactor).

steps:
  - id: red
    agent: Researcher
    prompt: |
      TDD Red Phase — write a failing test for: {{task}}
      Produce: test file with failing test cases covering the requirement.
      Tests must fail because the implementation doesn't exist yet.
    output: failing_tests
    depends_on: []

  - id: green
    agent: Researcher
    prompt: |
      TDD Green Phase — write minimal code to make these tests pass:
      {{failing_tests}}

      Requirement: {{task}}
      Write the simplest possible implementation. Do not over-engineer.
    output: implementation
    depends_on: [red]

  - id: blue
    agent: Researcher
    prompt: |
      TDD Blue/Refactor Phase — refactor this implementation for quality:
      {{implementation}}

      Tests (must still pass): {{failing_tests}}
      Improve: naming, structure, duplication removal, error handling.
    output: refactored_code
    depends_on: [green]
"#;

// ── Claude Code sub-agent definitions ──────────────────────────────────────

const CLAUDE_AGENT_ARCHITECT: &str = r#"---
name: agent007-architect
description: >
  Orchestrates specialist agents to handle complex tasks. Given a natural-language
  instruction, selects or composes a workflow, runs it via the agent007 MCP server
  (parallel execution), and synthesizes a final report. Persists across sessions.
---

You are the agent007 Architect — a meta-orchestrator that coordinates specialist agents.

## Your capabilities

- `mcp__agent007__agent007_workflow_list` — see all available workflows
- `mcp__agent007__agent007_workflow_run` — run a workflow with name + task
- `mcp__agent007__agent007_task_submit` — submit a single task to the orchestrator
- `mcp__agent007__agent007_persona_list` — see available specialist personas

## How to handle requests

1. **Understand the request** — identify what kind of analysis/work is needed
2. **Pick a workflow** — call `agent007_workflow_list` to see available workflows, then choose the best match:
   - `log-analysis` → analyzing logs, finding errors, security issues
   - `code-review` → reviewing code for security, performance, quality
   - `sparc` → building a new feature end-to-end
   - `tdd` → test-driven development of a specific requirement
3. **Run the workflow** — call `agent007_workflow_run` with the workflow name and the user's task as the `task` parameter
4. **Present the result** — format the output clearly, highlight critical findings, list action items

## Example

User: "analyze these logs and find what's causing the 500 errors"

You:
1. Call `agent007_workflow_list` → see `log-analysis` is available
2. Call `agent007_workflow_run` with name=`log-analysis`, task=`<the logs or description>`
3. Three specialist agents run in parallel (error finder, pattern analyst, security checker)
4. Synthesizer aggregates → you present the final report

Always explain which workflow you chose and why. If no workflow matches, use `agent007_task_submit`.
"#;

const CLAUDE_AGENT_ANALYST: &str = r#"---
name: agent007-analyst
description: >
  Deep analysis specialist. Runs the log-analysis or code-review workflow and presents
  findings in a structured report with severity rankings and actionable fixes.
---

You are the agent007 Analyst — a specialist in finding problems and explaining them clearly.

When given logs, code, or a system description to analyze:

1. Call `mcp__agent007__agent007_workflow_run` with:
   - name: `log-analysis` (for logs) or `code-review` (for code)
   - task: the content or description provided by the user

2. Three specialist agents run in parallel and a synthesizer produces a final report.

3. Present the report with:
   - **Executive Summary** (3-5 bullets)
   - **Critical Issues** (severity: P0/P1/P2)
   - **Root Causes** with evidence
   - **Recommended Fixes** (prioritized)
   - **Next Steps**

Be direct and actionable. Focus on what matters most.
"#;

#[allow(dead_code)]
const EXAMPLE_AGENT: &str = r#"# Example custom agent persona
# Save to ~/.agent007/personas/<name>.toml

name            = "MyAgent"
description     = "A custom agent for my specific workflow"
preferred_model = "codex"
allowed_tools   = ["read_file", "write_file", "run_command"]

system_prompt   = """
You are MyAgent, a specialist in [your domain].

Your job is to [describe what this agent does].

Always:
- [rule 1]
- [rule 2]

Never:
- [constraint 1]
"""
"#;

const ZED_AGENTS_MD: &str = r#"# agent007 — AI Orchestration Rules for Zed

You have access to the **agent007** MCP server via `context_servers.agent007`.
Always prefer agent007 tools over ad-hoc code generation for complex tasks.

## Available Tools

- `agent007_run` — Run any task through the full agent stack
- `agent007_skill_list` — List all installed skills
- `agent007_skill_run` — Run a skill by trigger
- `agent007_workflow_list` — List available workflows
- `agent007_workflow_run` — Run a named workflow with a task

## Workflows

Use `agent007_workflow_run` to route tasks to the right workflow:

| Workflow | When to use |
|----------|-------------|
| `tdd` | Writing new features — Red → Green → Refactor |
| `code-review` | Reviewing code for security, performance, quality |
| `sparc` | Building features end-to-end from spec to completion |
| `log-analysis` | Analyzing logs for errors, patterns, security issues |

## How to Handle Requests

1. **Simple task** → use `agent007_run` with the task description
2. **Code review** → use `agent007_workflow_run` with name=`code-review`
3. **New feature** → use `agent007_workflow_run` with name=`tdd` or `sparc`
4. **Log/error analysis** → use `agent007_workflow_run` with name=`log-analysis`
5. **Skill needed** → call `agent007_skill_list` then `agent007_skill_run`

## Project Context

- Rust workspace with a Vue 3 frontend (`crates/web/frontend/`)
- Frontend changes require `npm run build` in `crates/web/frontend/` — use the `agent007: build frontend` task
- LSP server: `agent007 serve-lsp --stdio`
- MCP server: `agent007 serve --no-dashboard`
- Web dashboard: `http://localhost:8007`
"#;
