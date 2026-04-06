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
    write_file(path, content, label, false)
}

fn write_file(path: &Path, content: &str, label: &str, force: bool) -> Result<bool> {
    if path.exists() && !force {
        ok(&format!("{label} already exists — skipped"));
        Ok(false)
    } else {
        std::fs::write(path, content)?;
        if force && path.exists() {
            ok(&format!("{label} updated (--force)"));
        } else {
            ok(&format!("{label} written"));
        }
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
    do_copilot: bool,
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
    let copilot_scope_dir = project_dir.join(".vscode");
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
    if do_copilot {
        ide_targets.push("Copilot (VS Code)");
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
        if write_file(&skills_dir_seed.join(filename), content, &format!("skills/{filename}"), force)? {
            built_in_skill_count += 1;
        }
    }
    if built_in_skill_count > 0 {
        ok(&format!("{built_in_skill_count} built-in skills seeded"));
    }

    // ── 4. Built-in workflows ───────────────────────────────────────────────
    section("4. Writing built-in workflows");
    let wf_dir = home.join("workflows");
    write_file(&wf_dir.join("log-analysis.yaml"),  WORKFLOW_LOG_ANALYSIS,  "workflows/log-analysis.yaml", force)?;
    write_file(&wf_dir.join("code-review.yaml"),   WORKFLOW_CODE_REVIEW,   "workflows/code-review.yaml",  force)?;
    write_file(&wf_dir.join("sparc.yaml"),         WORKFLOW_SPARC,         "workflows/sparc.yaml",        force)?;
    write_file(&wf_dir.join("tdd.yaml"),           WORKFLOW_TDD,           "workflows/tdd.yaml",          force)?;
    write_file(&wf_dir.join("ideation.yaml"),      WORKFLOW_IDEATION,      "workflows/ideation.yaml",     force)?;
    write_file(&wf_dir.join("feature.yaml"),       WORKFLOW_FEATURE,       "workflows/feature.yaml",      force)?;

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
        if write_file(&path, &content, &format!("personas/{filename}.toml"), force)? {
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

        // Install workflow slash commands alongside skill commands
        let workflows_dir = home.join("workflows");
        let mut wf_installed = 0usize;
        if let Ok(entries) = std::fs::read_dir(&workflows_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str());
                if ext != Some("yaml") && ext != Some("yml") {
                    continue;
                }
                let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                let cmd_file = commands_dir.join(format!("agent007-workflow-{stem}.md"));
                if !cmd_file.exists() || force {
                    // Read description from the YAML if present
                    let description = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|c| {
                            c.lines()
                                .find(|l| l.trim_start_matches(' ').starts_with("description:"))
                                .map(|l| l.trim_start_matches("description:").trim().trim_matches('>').trim().to_string())
                        })
                        .unwrap_or_else(|| format!("Run the {stem} workflow"));
                    let cmd_content = format!(
                        "{description}\n\nUse the mcp__agent007__agent007_workflow_run tool with name=\"{stem}\" and task=\"$ARGUMENTS\".\n"
                    );
                    std::fs::write(&cmd_file, cmd_content)?;
                    wf_installed += 1;
                }
            }
        }
        if wf_installed > 0 {
            ok(&format!("{wf_installed} workflow slash commands installed"));
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
        register_cursor_mcp(&cursor_scope_dir, "agent007", force)?;
        step += 1;
    }

    if do_codex {
        section(&format!("{step}. Registering MCP server with Codex"));
        register_codex_mcp(&codex_scope_dir, "agent007", force)?;
        step += 1;

        section(&format!("{step}. Installing agent007 skill into Codex"));
        install_codex_skill(force)?;
        step += 1;
    }

    if do_copilot {
        section(&format!("{step}. Registering MCP server with Copilot (VS Code)"));
        register_copilot_mcp(&copilot_scope_dir, "agent007", force)?;
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
    println!("  Workflows:   {GREEN}6{RESET} built-in (log-analysis, code-review, sparc, tdd, ideation, feature)");
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
    println!("{DIM}Quick start (from Claude Code / Cursor / Codex / Copilot):{RESET}");
    println!("  Use agent007 tools via MCP for hosted mode");
    println!("  Or configure Ollama / OPENAI_API_KEY / ANTHROPIC_API_KEY for standalone execution");
    println!("  Codex: MCP registered in .codex/config.toml, agent descriptions in .codex/agents.md");
    println!("  Copilot uses .vscode/mcp.json with servers.agent007");
    println!();

    Ok(())
}

/// Write (or merge) the agent007 MCP server entry into <claude_dir>/settings.json.
/// Uses `cmd` as the command path (preferably the currently running binary path).
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

    // Wire the statusLine so Claude Code shows live agent007 stats below the prompt.
    let obj = root.as_object_mut().unwrap();
    obj.entry("statusLine")
        .or_insert_with(|| serde_json::json!({
            "type": "command",
            "command": "cat ~/.agent007/statusline 2>/dev/null || echo 'agent007 | ready'"
        }));

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&settings_path, serde_json::to_string_pretty(&root)?)?;
    ok(&format!("Wrote mcpServers.agent007 → {cmd}"));
    ok("Wrote statusLine → cat ~/.agent007/statusline");
    ok(&format!("  config: {}", settings_path.display()));
    println!();
    warn("Restart Claude Code to activate the MCP server");
    info("New Claude Code windows will pick it up automatically");
    Ok(())
}

/// Write the agent007 MCP server entry into <cursor_dir>/mcp.json.
fn register_cursor_mcp(cursor_dir: &Path, cmd: &str, force: bool) -> Result<()> {
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
        "command": cmd,
        "args": ["serve"]
    });
    servers.insert("agent007".to_string(), entry);

    if let Some(parent) = mcp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&root)?)?;
    ok(&format!("Wrote mcpServers.agent007 → {cmd}"));
    ok(&format!("  config: {}", mcp_path.display()));
    println!();
    info("Restart Cursor to activate the MCP server");
    Ok(())
}

/// Write the agent007 MCP server entry into <vscode_dir>/mcp.json for GitHub Copilot.
///
/// VS Code MCP config uses a top-level "servers" map:
/// {
///   "servers": {
///     "agent007": {
///       "type": "stdio",
///       "command": "agent007",
///       "args": ["serve"]
///     }
///   }
/// }
fn register_copilot_mcp(vscode_dir: &Path, cmd: &str, force: bool) -> Result<()> {
    let mcp_path = vscode_dir.join("mcp.json");

    let mut root: serde_json::Value = if mcp_path.exists() {
        let raw = std::fs::read_to_string(&mcp_path)?;
        serde_json::from_str(&raw).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("servers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if servers.contains_key("agent007") && !force {
        ok(&format!("agent007 already registered in {}", mcp_path.display()));
        return Ok(());
    }

    let entry = serde_json::json!({
        "type": "stdio",
        "command": cmd,
        "args": ["serve"]
    });
    servers.insert("agent007".to_string(), entry);

    if let Some(parent) = mcp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&root)?)?;
    ok(&format!("Wrote servers.agent007 → stdio {cmd}"));
    ok(&format!("  config: {}", mcp_path.display()));
    println!();
    info("Restart VS Code / Copilot Chat to activate the MCP server");
    Ok(())
}

/// Write the agent007 MCP server entry into <codex_dir>/config.toml.
fn register_codex_mcp(codex_dir: &Path, cmd: &str, force: bool) -> Result<()> {
    let config_path = codex_dir.join("config.toml");

    let mut root: toml::Value = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)?;
        toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let table = root.as_table_mut().unwrap();

    let servers = table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .unwrap();

    if servers.contains_key("agent007") && !force {
        ok(&format!("agent007 already registered in {}", config_path.display()));
        return Ok(());
    }

    let mut entry = toml::map::Map::new();
    entry.insert("command".to_string(), toml::Value::String(cmd.to_string()));
    entry.insert(
        "args".to_string(),
        toml::Value::Array(vec![
            toml::Value::String("serve".to_string()),
            toml::Value::String("--no-dashboard".to_string()),
        ]),
    );
    servers.insert("agent007".to_string(), toml::Value::Table(entry));

    // Re-borrow root to insert `instructions` at top level.
    root.as_table_mut()
        .unwrap()
        .entry("instructions")
        .or_insert_with(|| toml::Value::String(CODEX_INSTRUCTIONS.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, toml::to_string_pretty(&root)?)?;
    ok(&format!("Wrote mcp_servers.agent007 → {cmd}"));
    ok(&format!("  config: {}", config_path.display()));
    println!();
    warn("Restart Codex to activate the MCP server");
    info("Codex uses `serve --no-dashboard` by default for strict stdio MCP mode");
    info("agent007 agents (architect, analyst) are described in the `instructions` field");
    Ok(())
}

/// Install agent007 as a Codex skill so it shows up under `/` commands and `@agent007`.
/// Skills live in ~/.codex/skills/<name>/ — always global, not project-scoped.
fn install_codex_skill(force: bool) -> Result<()> {
    let skill_dir = dirs_home().join(".codex").join("skills").join("agent007");
    let agents_dir = skill_dir.join("agents");
    std::fs::create_dir_all(&agents_dir)?;

    let skill_md = skill_dir.join("SKILL.md");
    let agent_yaml = agents_dir.join("openai.yaml");

    let mut wrote = 0usize;
    if write_file(&skill_md, CODEX_SKILL_MD, "~/.codex/skills/agent007/SKILL.md", force)? {
        wrote += 1;
    }
    if write_file(&agent_yaml, CODEX_SKILL_AGENT_YAML, "~/.codex/skills/agent007/agents/openai.yaml", force)? {
        wrote += 1;
    }

    if wrote > 0 {
        ok("agent007 skill installed — restart Codex then use @agent007 or /agent007");
    } else {
        ok("agent007 skill already installed (use --force to overwrite)");
    }
    Ok(())
}

/// Resolve the absolute path of the installed agent007 binary.
/// Falls back to the current executable path, then bare "agent007".
fn which_agent007() -> String {
    // Try `which agent007` first.
    if let Ok(out) = std::process::Command::new("which").arg("agent007").output() {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }
    // Fall back to the running executable.
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
    agent: DebugAgent
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
    agent: SecurityReviewer
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
    agent: SecurityReviewer
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
    agent: PerformanceEngineer
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
    agent: CodeReviewer
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
    agent: CodeReviewer
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
    agent: Coder
    prompt: |
      SPARC Phase 2 — Pseudocode.
      Based on this specification:
      {{specification}}

      Write structured pseudocode with logic flow, data structures, and algorithms.
    output: pseudocode
    depends_on: [spec]

  - id: architecture
    agent: Architect
    prompt: |
      SPARC Phase 3 — Architecture.
      Spec: {{specification}}
      Pseudocode: {{pseudocode}}

      Design the system architecture: components, interfaces, data flow, dependencies.
    output: architecture
    depends_on: [pseudocode]

  - id: refinement
    agent: CodeReviewer
    prompt: |
      SPARC Phase 4 — Refinement.
      Review the architecture for correctness, security, performance, and scalability.
      Architecture: {{architecture}}
      Identify and fix issues.
    output: refined_design
    depends_on: [architecture]

  - id: completion
    agent: ExpertCoder
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
    agent: TestDesigner
    prompt: |
      TDD Red Phase — write a failing test for: {{task}}
      Produce: test file with failing test cases covering the requirement.
      Tests must fail because the implementation doesn't exist yet.
    output: failing_tests
    depends_on: []

  - id: green
    agent: Coder
    prompt: |
      TDD Green Phase — write minimal code to make these tests pass:
      {{failing_tests}}

      Requirement: {{task}}
      Write the simplest possible implementation. Do not over-engineer.
    output: implementation
    depends_on: [red]

  - id: blue
    agent: ExpertCoder
    prompt: |
      TDD Blue/Refactor Phase — refactor this implementation for quality:
      {{implementation}}

      Tests (must still pass): {{failing_tests}}
      Improve: naming, structure, duplication removal, error handling.
    output: refactored_code
    depends_on: [green]
"#;

const WORKFLOW_IDEATION: &str = r#"name: ideation
description: >
  Ideation-to-plan pipeline. Research → human approval → documented ideation →
  PRD → architecture (reads PRD) → documented design → project planning →
  documented milestones. PRD drives architecture so requirements shape the design.

steps:
  - id: research
    agent: Researcher
    model: claude-haiku-4-5-20251001
    prompt: |
      You are a research specialist. Gather comprehensive context for:

      {{task}}

      Produce:
      1. Background and prior art
      2. Relevant patterns, technologies, and approaches
      3. Key constraints and risks
      4. Open questions that need human input
      5. Recommended direction with rationale

      Be thorough — this output drives all downstream steps.
    output: research_output
    depends_on: []

  - id: review-research
    agent: Researcher
    model: claude-sonnet-4-6
    requires_approval: true
    prompt: |
      Research is complete. Human: please review the findings and confirm
      the direction before architecture begins.

      RESEARCH OUTPUT:
      {{research_output}}

      Summarize the key decision points and open questions for the human reviewer.
    output: approval_notes
    depends_on: [research]

  - id: document-ideation
    agent: DocumentationWriter
    model: claude-haiku-4-5-20251001
    prompt: |
      You are a documentation specialist. Capture the ideation phase output
      as a structured document.

      TASK:
      {{task}}

      RESEARCH:
      {{research_output}}

      DIRECTION APPROVED:
      {{approval_notes}}

      Write: Ideation Document covering problem statement, goals, constraints,
      explored options, selected direction, and open questions.
    output: ideation_doc
    depends_on: [review-research]

  - id: prd
    agent: Planner
    model: claude-sonnet-4-6
    prompt: |
      You are a product manager. Write a Product Requirements Document for:

      {{task}}

      Based on:
      RESEARCH: {{research_output}}
      IDEATION: {{ideation_doc}}

      Include:
      - Executive summary
      - User stories and acceptance criteria
      - Functional and non-functional requirements
      - Out-of-scope items
      - Success metrics
    output: prd_doc
    depends_on: [document-ideation]

  - id: architecture
    agent: Architect
    model: claude-opus-4-6
    prompt: |
      You are a system architect. Design the technical architecture for:

      {{task}}

      Based on:
      RESEARCH: {{research_output}}
      IDEATION: {{ideation_doc}}
      PRD: {{prd_doc}}

      Deliver:
      - Component breakdown and responsibilities aligned to PRD requirements
      - Interface contracts and data flow
      - Technology choices with trade-offs
      - Non-functional requirements (perf, security, scalability)
      - Risk areas and mitigations
      - Module breakdown that maps to the features in the PRD
    output: architecture_doc
    depends_on: [prd]

  - id: document-design
    agent: DocumentationWriter
    model: claude-haiku-4-5-20251001
    prompt: |
      Capture the architecture and PRD into a unified Design Document.

      ARCHITECTURE:
      {{architecture_doc}}

      PRD:
      {{prd_doc}}

      Write: Design Document covering system design, component specs,
      API contracts, data models, and product requirements summary.
    output: design_doc
    depends_on: [architecture]

  - id: project-plan
    agent: Planner
    model: claude-sonnet-4-6
    prompt: |
      You are a project planner. Break the work into milestones and features.

      TASK: {{task}}
      DESIGN: {{design_doc}}

      Produce:
      - Milestones (ordered, with goals and exit criteria)
      - Features per milestone (description, dependencies, complexity estimate)
      - Suggested parallel workstreams
      - Definition of Done for the project
    output: project_plan
    depends_on: [document-design]

  - id: document-milestones
    agent: DocumentationWriter
    model: claude-haiku-4-5-20251001
    prompt: |
      Write the final Milestones & Features Document.

      PROJECT PLAN:
      {{project_plan}}

      Format: structured milestone table, feature breakdown per milestone,
      dependency graph narrative, and recommended execution order.
      This document will be handed to feature teams.
    output: milestones_doc
    depends_on: [project-plan]

  - id: write-project-docs
    agent: DocumentationWriter
    model: claude-haiku-4-5-20251001
    prompt: |
      The ideation phase is complete. Write the key outputs as real project
      documentation files so feature teams and future agents can reference them.

      Use the file write tool to create the following files in the project
      (create the docs/ directory if it does not exist):

      1. docs/prd.md
         Content: {{prd_doc}}

      2. docs/architecture.md
         Content: {{architecture_doc}}

      3. docs/design.md
         Content: {{design_doc}}

      4. docs/milestones.md
         Content: {{milestones_doc}}

      After writing all four files, produce a brief summary listing the files
      created and their purpose. These files should be committed to version
      control alongside the code.
    output: docs_written
    depends_on: [document-milestones]

  - id: repo-index
    agent: Researcher
    model: claude-haiku-4-5-20251001
    prompt: |
      The project documentation has been written to docs/. Index the repository
      so that all future agents can use semantic search to find relevant context.

      Call agent007_repo_brain_refresh to rebuild the vector index. This will
      index the newly written docs/ files alongside the existing codebase so
      that feature workflows can use agent007_context_compile to retrieve
      relevant context semantically rather than by fixed file paths.

      Report which directories were indexed and confirm the index is ready.
    output: index_confirmation
    depends_on: [write-project-docs]

  - id: present-and-approve
    agent: Planner
    model: claude-sonnet-4-6
    requires_approval: true
    prompt: |
      The ideation phase is complete. Present a full summary to the human
      for final approval before feature work begins.

      Produce a structured presentation covering:

      ## Project Summary
      What we are building and why (from {{prd_doc}}).

      ## Key Requirements
      Top 5-7 acceptance criteria and success metrics (from {{prd_doc}}).

      ## Architecture Overview
      Key components, technology choices, and major risks (from {{architecture_doc}}).

      ## Milestones & Features
      Milestone table with feature count and estimated complexity per milestone
      (from {{milestones_doc}}).

      ## Documentation Created
      - docs/prd.md
      - docs/architecture.md
      - docs/design.md
      - docs/milestones.md
      Index status: {{index_confirmation}}

      ## Next Steps
      How to start feature work:
        /agent007-workflow-feature <feature name from milestone 1>

      ---
      Human: please review the plan above.
      Approve to begin feature work, deny to halt, or edit to provide
      corrections that should be incorporated before proceeding.
    output: final_approval
    depends_on: [repo-index]
"#;

const WORKFLOW_FEATURE: &str = r#"name: feature
description: >
  Full-cycle feature delivery pipeline. Loads project context from ideation memory →
  research → feature spec (Planner) → architecture (reads spec) →
  implementation (full context) → human approval gate → parallel review
  (code, security, performance, gap, issues) → rework → test design →
  test coverage review → documentation → release sign-off (approval).

steps:
  - id: load-context
    agent: Researcher
    model: claude-haiku-4-5-20251001
    prompt: |
      Load project context for this feature using semantic search across the
      indexed repository, then supplement with any directly provided context.

      FEATURE REQUEST: {{task}}

      Step 1 — Semantic retrieval (preferred):
      Call agent007_context_compile with the feature description from {{task}}
      to retrieve the most relevant chunks from across the codebase and docs.
      This will surface relevant PRD sections, architecture patterns, existing
      code implementations, interface contracts, and milestone assignments
      without needing to know exact file paths.

      Step 2 — Fallback if index is empty:
      If agent007_context_compile returns no results, read these files directly:
      - docs/prd.md
      - docs/architecture.md
      - docs/design.md
      - docs/milestones.md

      Step 3 — Merge with task context:
      Combine the retrieved context with {{task}}. The explicit content in
      {{task}} takes precedence over retrieved context on any conflict —
      the caller can always override or supplement what the index provides.

      Produce a single "Project + Feature Context" document covering:
      1. Which milestone and feature slot this belongs to
      2. Relevant PRD requirements that this feature must satisfy
      3. Architectural constraints and patterns to follow
      4. Existing code patterns or interfaces to build on
      5. The specific feature request (from {{task}})
    output: project_context
    depends_on: []

  - id: research
    agent: Researcher
    model: claude-haiku-4-5-20251001
    prompt: |
      Research context for this feature:

      PROJECT CONTEXT:
      {{project_context}}

      Gather: existing code patterns, relevant APIs, constraints, prior art,
      edge cases, and integration points specific to this feature.
      Do not re-research what is already covered in the project context —
      focus on feature-specific details. Output a focused research brief.
    output: feature_research
    depends_on: [load-context]

  - id: document-brief
    agent: DocumentationWriter
    model: claude-haiku-4-5-20251001
    prompt: |
      Write a Feature Brief document.

      PROJECT CONTEXT: {{project_context}}
      FEATURE: {{task}}
      RESEARCH: {{feature_research}}

      Cover: feature scope, how it fits into the project milestones,
      acceptance criteria, technical approach summary, risks, and dependencies.
    output: feature_brief
    depends_on: [research]

  - id: feature-spec
    agent: Planner
    model: claude-sonnet-4-6
    prompt: |
      You are a product manager. Write a Feature Specification for:

      {{task}}

      Based on:
      PROJECT CONTEXT: {{project_context}}
      RESEARCH: {{feature_research}}
      BRIEF: {{feature_brief}}

      Include:
      - Precise acceptance criteria (testable, unambiguous)
      - Functional requirements (what the feature must do)
      - Non-functional requirements (performance, security, reliability)
      - Alignment with project PRD requirements (from project context)
      - Out-of-scope items
      - Edge cases and failure modes to handle
      - Definition of Done
    output: feature_spec
    depends_on: [document-brief]

  - id: architecture
    agent: Architect
    model: claude-opus-4-6
    prompt: |
      Design the technical approach for this feature.

      FEATURE: {{task}}
      RESEARCH: {{feature_research}}
      BRIEF: {{feature_brief}}
      SPEC: {{feature_spec}}

      Produce:
      - Component changes and new interfaces mapped to the spec requirements
      - Data model changes and migration plan
      - Integration points with existing systems
      - Non-functional design decisions (caching, error handling, concurrency)
      - Risks and open questions
    output: feature_architecture
    depends_on: [feature-spec]

  - id: implement
    agent: ExpertCoder
    model: gpt-5.3-codex
    prompt: |
      Implement the feature following the architecture and spec.

      FEATURE: {{task}}
      RESEARCH: {{feature_research}}
      SPEC: {{feature_spec}}
      ARCHITECTURE: {{feature_architecture}}

      Produce production-ready code with error handling, logging, and inline
      documentation. Follow existing code style and patterns.
      Every requirement in the spec must be addressed.
    output: implementation
    depends_on: [architecture]

  - id: review-implementation
    agent: CodeReviewer
    model: claude-sonnet-4-6
    requires_approval: true
    prompt: |
      Implementation is complete. Human: please review the implementation
      against the spec before running the full review pipeline.

      SPEC: {{feature_spec}}
      ARCHITECTURE: {{feature_architecture}}
      IMPLEMENTATION: {{implementation}}

      Summarize: does the implementation match the spec and architecture?
      Flag any obvious issues or direction changes needed before investing
      in the full review pipeline.
    output: review_notes
    depends_on: [implement]

  - id: code-review
    agent: CodeReviewer
    model: gpt-5.3-codex
    prompt: |
      Review the implementation for code quality.

      FEATURE: {{task}}
      SPEC: {{feature_spec}}
      IMPLEMENTATION: {{implementation}}

      Check: code smells, error handling, readability, maintainability,
      naming conventions, duplication. Output ranked findings.
    output: code_review_findings
    depends_on: [review-implementation]

  - id: security-review
    agent: SecurityReviewer
    model: gpt-5.3-codex
    prompt: |
      Security review of the implementation.

      FEATURE: {{task}}
      SPEC: {{feature_spec}}
      IMPLEMENTATION: {{implementation}}

      Check: injection vulnerabilities, auth flaws, insecure data handling,
      input validation, secrets management. Output ranked findings.
    output: security_findings
    depends_on: [review-implementation]

  - id: performance-review
    agent: PerformanceEngineer
    model: gpt-5.3-codex
    prompt: |
      Performance review of the implementation.

      FEATURE: {{task}}
      SPEC: {{feature_spec}}
      IMPLEMENTATION: {{implementation}}

      Check: algorithmic complexity, N+1 queries, blocking calls, memory usage,
      missing caching. Output ranked findings.
    output: performance_findings
    depends_on: [review-implementation]

  - id: gap-analysis
    agent: CodeReviewer
    model: gpt-5.3-codex
    prompt: |
      Gap analysis: compare implementation against the feature spec and architecture.

      FEATURE: {{task}}
      SPEC: {{feature_spec}}
      ARCHITECTURE: {{feature_architecture}}
      IMPLEMENTATION: {{implementation}}

      Identify: missing requirements, incomplete edge cases, unhandled errors,
      deviations from the architecture. Output a gap list.
    output: gap_findings
    depends_on: [review-implementation]

  - id: issue-analysis
    agent: DebugAgent
    model: gpt-5.3-codex
    prompt: |
      Issue analysis: identify potential bugs and failure modes.

      FEATURE: {{task}}
      SPEC: {{feature_spec}}
      IMPLEMENTATION: {{implementation}}

      Identify: logic errors, race conditions, null/panic risks, incorrect
      assumptions. Output a prioritized issue list.
    output: issue_findings
    depends_on: [review-implementation]

  - id: rework
    agent: ExpertCoder
    model: gpt-5.3-codex
    prompt: |
      Rework the implementation based on all review findings.

      SPEC: {{feature_spec}}
      ARCHITECTURE: {{feature_architecture}}
      ORIGINAL IMPLEMENTATION: {{implementation}}

      CODE REVIEW: {{code_review_findings}}
      SECURITY: {{security_findings}}
      PERFORMANCE: {{performance_findings}}
      GAPS: {{gap_findings}}
      ISSUES: {{issue_findings}}

      Produce the revised implementation addressing all findings.
      Ensure every spec requirement is still met after rework.
    output: revised_implementation
    depends_on: [code-review, security-review, performance-review, gap-analysis, issue-analysis]

  - id: test-design
    agent: TestDesigner
    model: gpt-5.3-codex
    prompt: |
      Design a comprehensive test suite for the feature.

      FEATURE: {{task}}
      SPEC: {{feature_spec}}
      IMPLEMENTATION: {{revised_implementation}}

      Write: unit tests, integration tests, edge case tests, regression tests.
      Map each test to a specific acceptance criterion in the spec.
      Cover all critical paths and identified failure modes.
    output: test_suite
    depends_on: [rework]

  - id: test-coverage-review
    agent: TestDesigner
    model: gpt-5.3-codex
    prompt: |
      Review the test suite for completeness and quality.

      SPEC: {{feature_spec}}
      IMPLEMENTATION: {{revised_implementation}}
      TEST SUITE: {{test_suite}}

      Assess: spec coverage (which criteria are covered/missing), edge case
      coverage, test reliability risks (flakiness, order-dependence),
      and test data requirements.
      Produce: coverage report with explicit pass/fail per acceptance criterion.
    output: test_report
    depends_on: [test-design]

  - id: document-feature
    agent: DocumentationWriter
    model: claude-haiku-4-5-20251001
    prompt: |
      Write the final feature documentation.

      FEATURE: {{task}}
      SPEC: {{feature_spec}}
      ARCHITECTURE: {{feature_architecture}}
      IMPLEMENTATION: {{revised_implementation}}
      TEST REPORT: {{test_report}}

      Produce: API documentation, usage examples, configuration guide,
      known limitations, and changelog entry.
    output: feature_docs
    depends_on: [test-coverage-review]

  - id: release-signoff
    agent: Planner
    model: claude-sonnet-4-6
    requires_approval: true
    prompt: |
      Feature delivery is complete. Prepare release sign-off summary.

      FEATURE: {{task}}
      SPEC: {{feature_spec}}
      ARCHITECTURE: {{feature_architecture}}
      IMPLEMENTATION: {{revised_implementation}}
      TEST REPORT: {{test_report}}
      DOCUMENTATION: {{feature_docs}}

      Write:
      - Spec compliance check (every acceptance criterion met?)
      - Release checklist
      - Go/no-go criteria assessment
      - Deployment notes and rollback plan
      - Monitoring recommendations
      Await human approval before marking as released.
    output: release_signoff
    depends_on: [document-feature]
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

const CODEX_INSTRUCTIONS: &str = r#"# agent007 — AI Orchestration Agents

You have access to the **agent007** MCP server. Use `mcp__agent007__*` tools for complex,
multi-step, or analytical tasks. Always prefer agent007 tools over ad-hoc generation.

## Available Agents

### agent007-architect
Orchestrates specialist agents for complex tasks. Selects a workflow, runs it via
the agent007 MCP server (parallel execution), and synthesizes a final report.

**When to use:** Multi-step features, system design, end-to-end delivery

```
mcp__agent007__agent007_workflow_list          → discover available workflows
mcp__agent007__agent007_workflow_run           → run a workflow (name + task)
mcp__agent007__agent007_task_submit            → submit task with persona="Architect"
```

### agent007-analyst
Deep analysis specialist. Runs log-analysis or code-review workflows and presents
findings in a structured report with severity rankings and actionable fixes.

**When to use:** Log analysis, code review, security audit, error investigation

```
mcp__agent007__agent007_workflow_run name=log-analysis   → analyze logs
mcp__agent007__agent007_workflow_run name=code-review    → review code
mcp__agent007__agent007_task_submit  persona="Analyst"   → direct analysis
```

## Quick Reference

| Task | MCP Tool |
|------|----------|
| Run any task | `mcp__agent007__agent007_run` |
| List workflows | `mcp__agent007__agent007_workflow_list` |
| Run a workflow | `mcp__agent007__agent007_workflow_run` |
| List skills | `mcp__agent007__agent007_skill_list` |
| Run a skill | `mcp__agent007__agent007_skill_run` |
| List personas/agents | `mcp__agent007__agent007_persona_list` |
| Submit a task | `mcp__agent007__agent007_task_submit` |

## Workflow Routing

| Workflow | When to use |
|----------|-------------|
| `tdd` | Writing new features — Red → Green → Refactor |
| `code-review` | Reviewing code for security, performance, quality |
| `sparc` | Building features end-to-end from spec to completion |
| `log-analysis` | Analyzing logs for errors, patterns, security issues |
| `feature` | Full-cycle feature delivery with approval gates |
| `ideation` | Research → PRD → architecture → project plan |
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

// ── Codex skill files ─────────────────────────────────────────────────────

const CODEX_SKILL_MD: &str = r#"---
name: agent007
description: >
  AI orchestration agent. Run multi-step workflows, code review, feature delivery,
  log analysis, TDD cycles, and complex tasks via the agent007 MCP server.
  Invoke with @agent007 or /agent007.
---

# agent007 — AI Orchestration

You have access to the **agent007** MCP server. Use `mcp__agent007__*` tools.

## Agents

### agent007-architect
Orchestrates specialist agents for complex tasks. Selects a workflow, runs it with
parallel execution, and synthesizes a final report.

**When to use:** Multi-step features, system design, end-to-end delivery

1. Call `mcp__agent007__agent007_workflow_list` to see available workflows
2. Call `mcp__agent007__agent007_workflow_run` with name + task
3. For direct tasks: `mcp__agent007__agent007_task_submit` with persona="Architect"

### agent007-analyst
Deep analysis specialist. Runs log-analysis or code-review workflows and presents
findings in a structured report with severity rankings and actionable fixes.

**When to use:** Log analysis, code review, security audit, error investigation

1. For logs: `mcp__agent007__agent007_workflow_run` with name=`log-analysis`
2. For code: `mcp__agent007__agent007_workflow_run` with name=`code-review`
3. Direct: `mcp__agent007__agent007_task_submit` with persona="Analyst"

## Workflow Routing

| Workflow | When to use |
|----------|-------------|
| `tdd` | Writing new features — Red → Green → Refactor |
| `code-review` | Reviewing code for security, performance, quality |
| `sparc` | Building features end-to-end from spec to completion |
| `log-analysis` | Analyzing logs for errors, patterns, security issues |
| `feature` | Full-cycle feature delivery with approval gates |
| `ideation` | Research → PRD → architecture → project plan |

## Quick Reference

| Task | MCP Tool |
|------|----------|
| Run any task | `mcp__agent007__agent007_run` |
| List workflows | `mcp__agent007__agent007_workflow_list` |
| Run a workflow | `mcp__agent007__agent007_workflow_run` |
| List skills | `mcp__agent007__agent007_skill_list` |
| Run a skill | `mcp__agent007__agent007_skill_run` |
| List personas | `mcp__agent007__agent007_persona_list` |
| Submit a task | `mcp__agent007__agent007_task_submit` |
"#;

const CODEX_SKILL_AGENT_YAML: &str = r#"interface:
  display_name: "agent007"
  short_description: "AI orchestration — workflows, code review, log analysis, TDD"
  default_prompt: "Use agent007 to run a workflow or task. Start with agent007_workflow_list to see what's available."

dependencies:
  tools:
    - type: "mcp"
      value: "agent007"
      description: "agent007 MCP orchestration server"
      transport: "stdio"
      command: "agent007"
      args: ["serve", "--no-dashboard"]
"#;
