use agent007_core::{
    build_and_save_index, write_repo_intelligence_readiness, RepoIntelligenceOptions,
};
use anyhow::{anyhow, Context, Result};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::path::{Path, PathBuf};

use super::run::{selected_runtime_model, selected_runtime_provider, standalone_mode_available};
use crate::config::Config;

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

fn command_slug(value: &str) -> String {
    value
        .trim_start_matches('/')
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

fn configured_asset_homes_for_init(home: &Path, global: bool) -> Vec<PathBuf> {
    let mut homes = Vec::new();
    if !global {
        homes.push(home.to_path_buf());
    }
    let global_home = super::run::agent007_global_home();
    if !homes.iter().any(|path| path == &global_home) {
        homes.push(global_home);
    }
    homes
}

fn available_skill_command_specs(home: &Path, global: bool) -> Vec<(String, String, String)> {
    let mut specs = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for skills_dir in configured_asset_homes_for_init(home, global)
        .into_iter()
        .flat_map(|agent_home| {
            let mut dirs = vec![agent_home.join("skills")];
            dirs.extend(agent007_core::paths::enabled_pack_asset_dirs(
                &agent_home,
                "skills",
            ));
            dirs
        })
    {
        if !skills_dir.exists() {
            continue;
        }
        let loader = agent007_skills::SkillLoader::new(&skills_dir);
        let Ok(skills) = loader.load_all() else {
            continue;
        };
        for skill in skills {
            let trigger = skill.trigger().to_string();
            if !seen.insert(trigger.clone()) {
                continue;
            }
            let slug = command_slug(&trigger);
            specs.push((slug, skill.frontmatter.description.clone(), trigger));
        }
    }

    specs
}

fn available_workflow_command_specs(home: &Path, global: bool) -> Vec<(String, String)> {
    let mut specs = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for workflows_dir in configured_asset_homes_for_init(home, global)
        .into_iter()
        .flat_map(|agent_home| {
            let mut dirs = vec![agent_home.join("workflows")];
            dirs.extend(agent007_core::paths::enabled_pack_asset_dirs(
                &agent_home,
                "workflows",
            ));
            dirs
        })
    {
        if !workflows_dir.exists() {
            continue;
        }
        let loader = agent007_workflows::WorkflowLoader::new(workflows_dir);
        let Ok(names) = loader.list_names() else {
            continue;
        };
        for name in names {
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Ok(def) = loader.load_named(&name) {
                specs.push((name, def.description.unwrap_or_default()));
            }
        }
    }

    specs
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardMode {
    ServedByMcpProcess,
    SeparateProcess,
}

#[derive(Debug, Clone)]
struct IntegrationCheck {
    label: &'static str,
    config_path: PathBuf,
    command: String,
    args: Vec<String>,
    dashboard_mode: DashboardMode,
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

const WORKFLOW_OPERATING_PROTOCOL_MARKER: &str = "{{WORKFLOW_OPERATING_PROTOCOL}}";

const WORKFLOW_OPERATING_PROTOCOL: &str = "\
Operating protocol:
      - Identify this step's goal, success criteria, inputs, constraints, and expected downstream handoff before producing the output.
      - Reason stepwise internally, but do not expose private chain-of-thought; return concise rationale, key trade-offs, and decision criteria.
      - Build an evidence ledger before making claims: files inspected, commands run, ETR/tool outputs, source citations, upstream step IDs, and confidence level.
      - Prefer agent007 ETR tools for grep/glob/file stats, JSON/table/log queries, metrics, diffs, and workflow status before ad-hoc shell parsing when available.
      - Use shell/build/test tools for execution and verification, not for noisy parsing that ETR can perform deterministically.
      - Separate facts, inferences, assumptions, and recommendations. If required context is missing, state the assumption and choose a reversible, low-risk path.
      - Stay within this step's role; name handoffs for other agents instead of expanding scope silently.
      - Return concrete findings, decisions, risks, validation, and next actions; avoid generic checklist filler.
      - Do not claim validation ran unless it actually ran; otherwise name the exact validation to run.
      - When multiple options are plausible, compare them with explicit criteria and recommend one default path.";

fn render_builtin_workflow_template(template: &str) -> String {
    template.replace(
        WORKFLOW_OPERATING_PROTOCOL_MARKER,
        WORKFLOW_OPERATING_PROTOCOL,
    )
}

fn write_workflow_file(path: &Path, template: &str, label: &str, force: bool) -> Result<bool> {
    let rendered = render_builtin_workflow_template(template);
    write_file(path, &rendered, label, force)
}

fn write_workflow_if_missing(path: &Path, template: &str, label: &str) -> Result<bool> {
    let rendered = render_builtin_workflow_template(template);
    write_if_missing(path, &rendered, label)
}

fn backup_path_for(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("cannot create backup path for {}", path.display()))?;
    let mut candidate = path.with_file_name(format!("{file_name}.agent007.bak"));
    let mut index = 1usize;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{file_name}.agent007.bak.{index}"));
        index += 1;
    }
    Ok(candidate)
}

fn backup_existing_file(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }

    let backup = backup_path_for(path)?;
    std::fs::copy(path, &backup).with_context(|| {
        format!(
            "failed to back up existing file {} -> {}",
            path.display(),
            backup.display()
        )
    })?;
    Ok(Some(backup))
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("{} has no file name", path.display()))?;
    let tmp = parent.join(format!(".{file_name}.agent007.tmp"));
    std::fs::write(&tmp, content)?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn write_file(path: &Path, content: &str, label: &str, force: bool) -> Result<bool> {
    if path.exists() && !force {
        ok(&format!("{label} already exists — skipped"));
        Ok(false)
    } else {
        let existed = path.exists();
        if existed {
            let current = std::fs::read_to_string(path).unwrap_or_default();
            if current == content {
                ok(&format!("{label} already up to date"));
                return Ok(false);
            }
            if let Some(backup) = backup_existing_file(path)? {
                info(&format!("backup written: {}", backup.display()));
            }
        }
        atomic_write(path, content)?;
        if existed {
            ok(&format!("{label} updated"));
        } else {
            ok(&format!("{label} written"));
        }
        Ok(true)
    }
}

fn load_json_root(path: &Path, label: &str) -> Result<JsonValue> {
    if !path.exists() {
        return Ok(JsonValue::Object(JsonMap::new()));
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {label} at {}", path.display()))?;
    let parsed: JsonValue = serde_json::from_str(&raw).map_err(|error| {
        anyhow!(
            "failed to parse {label} at {}: {error}. Refusing to overwrite an existing file.",
            path.display()
        )
    })?;
    if !parsed.is_object() {
        return Err(anyhow!(
            "{label} at {} must be a JSON object. Refusing to overwrite an existing file.",
            path.display()
        ));
    }
    Ok(parsed)
}

fn write_json_root(path: &Path, root: &JsonValue, label: &str) -> Result<()> {
    let serialized = serde_json::to_string_pretty(root)?;
    if path.exists() && std::fs::read_to_string(path).unwrap_or_default() == serialized {
        ok(&format!("{label} already up to date"));
        ok(&format!("  config: {}", path.display()));
        return Ok(());
    }
    if let Some(backup) = backup_existing_file(path)? {
        info(&format!("backup written: {}", backup.display()));
    }
    atomic_write(path, &serialized)?;
    ok(&format!("Wrote {label}"));
    ok(&format!("  config: {}", path.display()));
    Ok(())
}

fn load_toml_root(path: &Path, label: &str) -> Result<toml::Value> {
    if !path.exists() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {label} at {}", path.display()))?;
    let parsed: toml::Value = toml::from_str(&raw).map_err(|error| {
        anyhow!(
            "failed to parse {label} at {}: {error}. Refusing to overwrite an existing file.",
            path.display()
        )
    })?;
    if !parsed.is_table() {
        return Err(anyhow!(
            "{label} at {} must be a TOML table. Refusing to overwrite an existing file.",
            path.display()
        ));
    }
    Ok(parsed)
}

fn write_toml_root(path: &Path, root: &toml::Value, label: &str) -> Result<()> {
    let serialized = toml::to_string_pretty(root)?;
    if path.exists() && std::fs::read_to_string(path).unwrap_or_default() == serialized {
        ok(&format!("{label} already up to date"));
        ok(&format!("  config: {}", path.display()));
        return Ok(());
    }
    if let Some(backup) = backup_existing_file(path)? {
        info(&format!("backup written: {}", backup.display()));
    }
    atomic_write(path, &serialized)?;
    ok(&format!("Wrote {label}"));
    ok(&format!("  config: {}", path.display()));
    Ok(())
}

fn dashboard_mode_for_args(args: &[String]) -> DashboardMode {
    if args.iter().any(|arg| arg == "--no-dashboard") {
        DashboardMode::SeparateProcess
    } else {
        DashboardMode::ServedByMcpProcess
    }
}

fn verify_binary_target(cmd: &str) {
    let path = Path::new(cmd);
    if path.is_absolute() {
        if path.exists() {
            ok(&format!("agent007 binary resolved → {}", path.display()));
        } else {
            warn(&format!(
                "configured binary path does not exist → {}",
                path.display()
            ));
        }
    } else {
        info(&format!("agent007 binary resolved via PATH lookup → {cmd}"));
    }
}

fn parse_json_command_entry(
    path: &Path,
    label: &'static str,
    top_level_key: &str,
) -> Result<IntegrationCheck> {
    let root = load_json_root(path, label)?;
    let entry = root
        .get(top_level_key)
        .and_then(|value| value.get("agent007"))
        .and_then(|value| value.as_object())
        .ok_or_else(|| anyhow!("{label} missing agent007 entry at {}", path.display()))?;

    let command = entry
        .get("command")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("{label} missing command at {}", path.display()))?
        .to_string();
    let args = entry
        .get("args")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("{label} missing args at {}", path.display()))?
        .iter()
        .filter_map(|value| value.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();

    Ok(IntegrationCheck {
        label,
        config_path: path.to_path_buf(),
        dashboard_mode: dashboard_mode_for_args(&args),
        command,
        args,
    })
}

fn parse_toml_command_entry(
    path: &Path,
    label: &'static str,
    top_level_key: &str,
) -> Result<IntegrationCheck> {
    let root = load_toml_root(path, label)?;
    let entry = root
        .get(top_level_key)
        .and_then(|value| value.get("agent007"))
        .and_then(|value| value.as_table())
        .ok_or_else(|| anyhow!("{label} missing agent007 entry at {}", path.display()))?;

    let command = entry
        .get("command")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("{label} missing command at {}", path.display()))?
        .to_string();
    let args = entry
        .get("args")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("{label} missing args at {}", path.display()))?
        .iter()
        .filter_map(|value| value.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();

    Ok(IntegrationCheck {
        label,
        config_path: path.to_path_buf(),
        dashboard_mode: dashboard_mode_for_args(&args),
        command,
        args,
    })
}

fn print_integration_check(check: &IntegrationCheck) {
    ok(&format!(
        "{} MCP entry verified → {} {}",
        check.label,
        check.command,
        check.args.join(" ")
    ));
    info(&format!("config: {}", check.config_path.display()));
    match check.dashboard_mode {
        DashboardMode::ServedByMcpProcess => {
            info("dashboard mode: same process as MCP (`serve`) — the web UI should come up with the editor-launched MCP server");
        }
        DashboardMode::SeparateProcess => {
            info("dashboard mode: separate process (`--no-dashboard`) — start `agent007 dashboard` or `agent007 serve` manually if you want the web UI");
        }
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
    let binary_path = which_agent007();
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
    let scope_label = if global {
        "~/.agent007/ (global)"
    } else {
        ".agent007/ (project-local)"
    };
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
    ensure_dir(&home.join("runtime"), "runtime/")?;
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
        if write_file(
            &skills_dir_seed.join(filename),
            content,
            &format!("skills/{filename}"),
            force,
        )? {
            built_in_skill_count += 1;
        }
    }
    if built_in_skill_count > 0 {
        ok(&format!("{built_in_skill_count} built-in skills seeded"));
    }

    // ── 4. Built-in workflows ───────────────────────────────────────────────
    section("4. Writing built-in workflows");
    let wf_dir = home.join("workflows");
    write_workflow_file(
        &wf_dir.join("log-analysis.yaml"),
        WORKFLOW_LOG_ANALYSIS,
        "workflows/log-analysis.yaml",
        force,
    )?;
    write_workflow_file(
        &wf_dir.join("code-review.yaml"),
        WORKFLOW_CODE_REVIEW,
        "workflows/code-review.yaml",
        force,
    )?;
    write_workflow_file(
        &wf_dir.join("security-audit.yaml"),
        WORKFLOW_SECURITY_AUDIT,
        "workflows/security-audit.yaml",
        force,
    )?;
    write_workflow_file(
        &wf_dir.join("sparc.yaml"),
        WORKFLOW_SPARC,
        "workflows/sparc.yaml",
        force,
    )?;
    write_workflow_file(
        &wf_dir.join("tdd.yaml"),
        WORKFLOW_TDD,
        "workflows/tdd.yaml",
        force,
    )?;
    write_workflow_file(
        &wf_dir.join("ideation.yaml"),
        WORKFLOW_IDEATION,
        "workflows/ideation.yaml",
        force,
    )?;
    write_workflow_file(
        &wf_dir.join("feature.yaml"),
        WORKFLOW_FEATURE,
        "workflows/feature.yaml",
        force,
    )?;
    write_workflow_file(
        &wf_dir.join("brainstorm.yaml"),
        WORKFLOW_BRAINSTORM,
        "workflows/brainstorm.yaml",
        force,
    )?;
    write_workflow_file(
        &wf_dir.join("llm-council.yaml"),
        WORKFLOW_LLM_COUNCIL,
        "workflows/llm-council.yaml",
        force,
    )?;

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
        let filename = spec
            .name
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
        let tools_str = spec
            .allowed_tools
            .iter()
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

    // ── 5b. Bootstrap global ~/.agent007/ if this is a project-local init ───
    if !global {
        let global_home = super::run::agent007_global_home();
        section("5b. Bootstrapping global ~/.agent007/ (first-run)");
        if let Err(e) = seed_global_if_missing(&global_home) {
            warn(&format!("Could not seed global home: {e}"));
        }

        section("5c. Building initial repo index");
        match build_and_save_index(&project_dir, None) {
            Ok(status) => {
                let counts = status.counts.unwrap_or_default();
                ok(&format!(
                    "repo index created ({} files, {} symbols)",
                    counts.files, counts.symbols
                ));
            }
            Err(e) => warn(&format!("Could not build initial repo index: {e}")),
        }
        match write_repo_intelligence_readiness(
            &project_dir,
            None,
            &RepoIntelligenceOptions::default(),
        ) {
            Ok(readiness) => ok(&format!("repo readiness captured ({:?})", readiness.state)),
            Err(e) => warn(&format!("Could not capture repo readiness: {e}")),
        }
    }

    // ── 6. IDE integrations ─────────────────────────────────────────────────
    let mut step = 6;

    if do_claude {
        section(&format!("{step}. Registering MCP server with Claude Code"));
        register_claude_mcp(&binary_path, &claude_scope_dir, &project_dir, global, force)?;
        step += 1;

        if !global {
            section(&format!("{step}. Writing Claude Code project guidance"));
            register_claude_project_files(&project_dir, &claude_scope_dir, force)?;
            step += 1;
        }

        section(&format!(
            "{step}. Installing slash commands for Claude Code"
        ));
        let commands_dir = claude_scope_dir.join("commands");
        if !commands_dir.exists() {
            std::fs::create_dir_all(&commands_dir)?;
            ok("commands/ created");
        }

        let mut installed = 0usize;
        for (slug, description, trigger) in available_skill_command_specs(&home, global) {
            let cmd_file = commands_dir.join(format!("agent007-{slug}.md"));
            if !cmd_file.exists() || force {
                let cmd_content = format!(
                    "{description}\n\nUse the mcp__agent007__agent007_skill_run tool with trigger \"{trigger}\" and args \"$ARGUMENTS\".\n"
                );
                if write_file(
                    &cmd_file,
                    &cmd_content,
                    &format!("commands/agent007-{slug}.md"),
                    force,
                )? {
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
        let mut wf_installed = 0usize;
        for (name, description) in available_workflow_command_specs(&home, global) {
            let cmd_file = commands_dir.join(format!("agent007-workflow-{name}.md"));
            if !cmd_file.exists() || force {
                let cmd_content = format!(
                    "{}\n\nUse the mcp__agent007__agent007_workflow_run tool with name=\"{}\" and task=\"$ARGUMENTS\".\n",
                    if description.is_empty() {
                        format!("Run the {name} workflow")
                    } else {
                        description
                    },
                    name,
                );
                if write_file(
                    &cmd_file,
                    &cmd_content,
                    &format!("commands/agent007-workflow-{name}.md"),
                    force,
                )? {
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
        for (filename, content) in claude_agent_specs() {
            write_file(
                &agents_dir.join(filename),
                content,
                &format!("agents/{filename}"),
                force,
            )?;
        }
        step += 1;
    }

    if do_cursor {
        section(&format!("{step}. Registering MCP server with Cursor"));
        register_cursor_mcp(&cursor_scope_dir, &binary_path, force)?;
        step += 1;
        if !global {
            section(&format!("{step}. Writing Cursor project rules"));
            register_cursor_project_files(&project_dir, force)?;
            step += 1;
        }
    }

    if do_codex {
        section(&format!("{step}. Registering MCP server with Codex"));
        register_codex_mcp(&codex_scope_dir, &binary_path, force)?;
        step += 1;

        if !global {
            section(&format!("{step}. Writing Codex project guidance"));
            register_codex_project_files(&project_dir, force)?;
            step += 1;
        }

        section(&format!("{step}. Installing agent007 skill into Codex"));
        install_codex_skill(&binary_path, force)?;
        step += 1;
    }

    if do_copilot {
        section(&format!(
            "{step}. Registering MCP server with Copilot (VS Code)"
        ));
        register_copilot_mcp(&copilot_scope_dir, &binary_path, force)?;
        step += 1;
        if !global {
            section(&format!(
                "{step}. Writing shared workspace MCP config for Copilot CLI"
            ));
            register_workspace_mcp_json(&binary_path, &project_dir, force)?;
            step += 1;

            section(&format!(
                "{step}. Writing GitHub Copilot project instructions"
            ));
            register_copilot_project_files(&project_dir, force)?;
            step += 1;
        }
    }

    if do_zed {
        section(&format!("{step}. Registering LSP server with Zed"));
        let zed_scope_dir = if global {
            dirs_home().join(".config").join("zed")
        } else {
            project_dir.join(".zed")
        };
        register_zed(&zed_scope_dir, &project_dir, global, force)?;
        step += 1;
    }

    section(&format!("{step}. Verifying editor integration wiring"));
    verify_binary_target(&binary_path);
    if do_claude {
        let claude_mcp_path = if global {
            dirs_home().join(".claude.json")
        } else {
            project_dir.join(".mcp.json")
        };
        let claude_mcp_label = if global {
            "~/.claude.json"
        } else {
            ".mcp.json"
        };
        let check = parse_json_command_entry(&claude_mcp_path, claude_mcp_label, "mcpServers")?;
        print_integration_check(&check);
    }
    if do_cursor {
        let check = parse_json_command_entry(
            &cursor_scope_dir.join("mcp.json"),
            "Cursor mcp.json",
            "mcpServers",
        )?;
        print_integration_check(&check);
    }
    if do_codex {
        let check = parse_toml_command_entry(
            &codex_scope_dir.join("config.toml"),
            "Codex config.toml",
            "mcp_servers",
        )?;
        print_integration_check(&check);
    }
    if do_copilot {
        let check = parse_json_command_entry(
            &copilot_scope_dir.join("mcp.json"),
            "VS Code mcp.json",
            "servers",
        )?;
        print_integration_check(&check);
    }
    if do_zed {
        let zed_scope_dir = if global {
            dirs_home().join(".config").join("zed")
        } else {
            project_dir.join(".zed")
        };
        let check = parse_json_command_entry(
            &zed_scope_dir.join("settings.json"),
            "Zed settings.json",
            "context_servers",
        )?;
        print_integration_check(&check);
    }
    step += 1;

    // ── 9. Environment check ───────────────────────────────────────────────
    section(&format!("{step}. Environment check"));

    let anthropic_key = super::credentials::get("anthropic").unwrap_or_default();
    let openai_key = super::credentials::get("openai").unwrap_or_default();
    if anthropic_key.is_empty() {
        info("Anthropic credential not configured");
    } else {
        ok(&format!(
            "Anthropic credential available via {} ({} chars)",
            super::credentials::source("anthropic").unwrap_or("configured source"),
            anthropic_key.len(),
        ));
    }
    if openai_key.is_empty() {
        info("OpenAI credential not configured");
    } else {
        ok(&format!(
            "OpenAI credential available via {} ({} chars)",
            super::credentials::source("openai").unwrap_or("configured source"),
            openai_key.len(),
        ));
    }
    if standalone_mode_available(&config) {
        let provider = selected_runtime_provider(&config).unwrap_or_else(|| "unknown".to_string());
        let model = selected_runtime_model(&config).unwrap_or_else(|| "unknown".to_string());
        ok(&format!(
            "standalone mode available via {provider} ({model})"
        ));
    } else {
        info("No standalone provider configured — hosted MCP mode active (host LLM handles reasoning via MCP)");
        info("Set OPENAI_API_KEY, ANTHROPIC_API_KEY, or [models.ollama] for standalone mode");
    }

    let git_ok = std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok();
    if git_ok {
        ok("git available")
    } else {
        warn("git not found in PATH")
    }

    step += 1;

    // ── 10. Summary ────────────────────────────────────────────────────────
    let skill_count = available_skill_command_specs(&home, global).len();
    let persona_count = {
        let mut dirs = Vec::new();
        for agent_home in configured_asset_homes_for_init(&home, global) {
            dirs.push(agent_home.join("personas"));
            dirs.extend(agent007_core::paths::enabled_pack_asset_dirs(
                &agent_home,
                "personas",
            ));
        }
        dirs.reverse();
        let registry = agent007_personas::PersonaRegistry::load_from_dirs(
            dirs.iter().map(|dir| dir.as_path()),
        )
        .unwrap_or_else(|_| agent007_personas::PersonaRegistry::built_in());
        use agent007_core::PersonaProvider;
        registry.list().len()
    };
    let workflow_count = available_workflow_command_specs(&home, global).len();

    println!();
    section(&format!("{step}. Summary"));
    println!("{BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}");
    println!("{BOLD}{GREEN}agent007 is ready!{RESET}");
    println!("{BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}");
    println!();
    println!("  Home:        {DIM}{}{RESET}", home.display());
    println!("  Personas:    {GREEN}{}{RESET} available", persona_count);
    println!("  Skills:      {GREEN}{skill_count}{RESET} loaded");
    println!("  Workflows:   {GREEN}{workflow_count}{RESET} available");
    println!("  MCP server:  {GREEN}agent007 serve{RESET}");
    println!("  Dashboard:   {CYAN}http://localhost:8007{RESET} (served by `agent007 serve` when the dashboard is enabled)");
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
    println!(
        "  Codex: MCP registered in .codex/config.toml, agent descriptions in .codex/agents.md"
    );
    println!("  Copilot uses .vscode/mcp.json with servers.agent007");
    println!();

    Ok(())
}

/// Write (or merge) the agent007 MCP server entry into <claude_dir>/settings.json.
/// Uses `cmd` as the command path (preferably the currently running binary path).
fn register_claude_mcp(
    cmd: &str,
    claude_dir: &Path,
    project_dir: &Path,
    global: bool,
    force: bool,
) -> Result<()> {
    // Claude Code now separates MCP config from settings.json:
    //   - User/global scope  → ~/.claude.json  (mcpServers at root)
    //   - Project scope      → <project>/.mcp.json  (mcpServers at root, committed to git)
    let mcp_path = if global {
        dirs_home().join(".claude.json")
    } else {
        project_dir.join(".mcp.json")
    };
    let file_label = if global {
        "~/.claude.json"
    } else {
        ".mcp.json"
    };

    let mut root = load_json_root(&mcp_path, file_label)?;

    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    let desired_entry = serde_json::json!({
        "command": cmd,
        "args": ["serve"]
    });
    if let Some(existing) = servers.get("agent007").cloned() {
        if force {
            servers.insert("agent007".to_string(), desired_entry);
        } else {
            let existing_command = existing
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let existing_args: Vec<String> = existing
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|value| value.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let has_no_dashboard_flag = existing_args.iter().any(|arg| arg == "--no-dashboard");
            if has_no_dashboard_flag || existing_args != vec!["serve".to_string()] {
                let command_to_keep = if existing_command.is_empty() {
                    cmd.to_string()
                } else {
                    existing_command
                };
                let migrated = serde_json::json!({
                    "command": command_to_keep,
                    "args": ["serve"]
                });
                servers.insert("agent007".to_string(), migrated);
                ok("Updated existing Claude MCP entry to `agent007 serve` (dashboard-enabled)");
            } else {
                ok(&format!(
                    "agent007 already registered in {}",
                    mcp_path.display()
                ));
            }
        }
    } else {
        servers.insert("agent007".to_string(), desired_entry);
    }

    if let Some(parent) = mcp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_json_root(&mcp_path, &root, &format!("mcpServers.agent007 → {cmd}"))?;
    ok(&format!("MCP server registered in {file_label}"));

    // Update settings.json: migrate out any old mcpServers.agent007 entry (now in
    // the new location above), and wire the statusLine so Claude Code shows live
    // agent007 stats below the prompt.
    let settings_path = claude_dir.join("settings.json");
    let mut settings = load_json_root(&settings_path, "Claude Code settings.json")?;
    let settings_obj = settings.as_object_mut().unwrap();

    // Remove stale mcpServers.agent007 written by older versions of agent007 init
    if let Some(mcp) = settings_obj
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
    {
        if mcp.remove("agent007").is_some() {
            info("Migrated: removed old mcpServers.agent007 from settings.json");
        }
    }

    settings_obj.entry("statusLine").or_insert_with(|| {
        serde_json::json!({
            "type": "command",
            "command": "cat ~/.agent007/statusline 2>/dev/null || echo 'agent007 | ready'"
        })
    });

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_json_root(
        &settings_path,
        &settings,
        &format!("statusLine → ~/.agent007/statusline"),
    )?;
    ok("Ensured statusLine → cat ~/.agent007/statusline");

    println!();
    warn("Restart Claude Code to activate the MCP server");
    info("New Claude Code windows will pick it up automatically");
    Ok(())
}

fn register_workspace_mcp_json(cmd: &str, project_dir: &Path, force: bool) -> Result<()> {
    let mcp_path = project_dir.join(".mcp.json");
    let mut root = load_json_root(&mcp_path, ".mcp.json")?;
    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if servers.contains_key("agent007") && !force {
        ok(&format!(
            "agent007 already registered in {}",
            mcp_path.display()
        ));
    } else {
        servers.insert(
            "agent007".to_string(),
            serde_json::json!({
                "command": cmd,
                "args": ["serve"]
            }),
        );
    }

    if let Some(parent) = mcp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_json_root(&mcp_path, &root, &format!("mcpServers.agent007 → {cmd}"))?;
    Ok(())
}

/// Write the agent007 MCP server entry into <cursor_dir>/mcp.json.
fn register_cursor_mcp(cursor_dir: &Path, cmd: &str, force: bool) -> Result<()> {
    let mcp_path = cursor_dir.join("mcp.json");

    let mut root = load_json_root(&mcp_path, "Cursor mcp.json")?;

    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if servers.contains_key("agent007") && !force {
        ok(&format!(
            "agent007 already registered in {}",
            mcp_path.display()
        ));
    } else {
        let entry = serde_json::json!({
            "command": cmd,
            "args": ["serve"]
        });
        servers.insert("agent007".to_string(), entry);
    }

    if let Some(parent) = mcp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_json_root(&mcp_path, &root, &format!("mcpServers.agent007 → {cmd}"))?;
    println!();
    info("Restart Cursor to activate the MCP server");
    Ok(())
}

/// Write the agent007 MCP server entry into <vscode_dir>/mcp.json for
/// VS Code / Copilot Chat.
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
///
/// Note: GitHub Copilot CLI now prefers project `.mcp.json` / `.github/mcp.json`
/// and user `~/.copilot/mcp-config.json`. Project-level `.mcp.json` is already
/// written by the Claude/Copilot shared MCP path in project init.
fn register_copilot_mcp(vscode_dir: &Path, cmd: &str, force: bool) -> Result<()> {
    let mcp_path = vscode_dir.join("mcp.json");

    let mut root = load_json_root(&mcp_path, "VS Code mcp.json")?;

    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("servers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if servers.contains_key("agent007") && !force {
        ok(&format!(
            "agent007 already registered in {}",
            mcp_path.display()
        ));
    } else {
        let entry = serde_json::json!({
            "type": "stdio",
            "command": cmd,
            "args": ["serve"]
        });
        servers.insert("agent007".to_string(), entry);
    }

    if let Some(parent) = mcp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_json_root(&mcp_path, &root, &format!("servers.agent007 → stdio {cmd}"))?;
    println!();
    info("Restart VS Code / Copilot Chat to activate the MCP server");
    Ok(())
}

/// Write the agent007 MCP server entry into <codex_dir>/config.toml.
fn register_codex_mcp(codex_dir: &Path, cmd: &str, force: bool) -> Result<()> {
    let config_path = codex_dir.join("config.toml");

    let mut root = load_toml_root(&config_path, "Codex config.toml")?;

    let table = root.as_table_mut().unwrap();

    let servers = table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .unwrap();

    if servers.contains_key("agent007") && !force {
        ok(&format!(
            "agent007 already registered in {}",
            config_path.display()
        ));
    } else {
        let mut entry = toml::map::Map::new();
        entry.insert("command".to_string(), toml::Value::String(cmd.to_string()));
        entry.insert(
            "args".to_string(),
            toml::Value::Array(vec![toml::Value::String("serve".to_string())]),
        );
        servers.insert("agent007".to_string(), toml::Value::Table(entry));
    }

    // Re-borrow root to insert `instructions` at top level.
    root.as_table_mut()
        .unwrap()
        .entry("instructions")
        .or_insert_with(|| toml::Value::String(CODEX_INSTRUCTIONS.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_toml_root(
        &config_path,
        &root,
        &format!("mcp_servers.agent007 → {cmd}"),
    )?;
    println!();
    warn("Restart Codex to activate the MCP server");
    info("agent007 coordinator + worker guidance is described in the `instructions` field");
    Ok(())
}

/// Install agent007 as a Codex skill so it shows up under `/` commands and `@agent007`.
/// Skills live in ~/.codex/skills/<name>/ — always global, not project-scoped.
fn install_codex_skill(cmd: &str, force: bool) -> Result<()> {
    let skill_dir = dirs_home().join(".codex").join("skills").join("agent007");
    let agents_dir = skill_dir.join("agents");
    std::fs::create_dir_all(&agents_dir)?;

    let skill_md = skill_dir.join("SKILL.md");
    let agent_yaml = agents_dir.join("openai.yaml");
    let skill_agent_yaml = format!(
        "interface:\n  display_name: \"agent007\"\n  short_description: \"AI orchestration — workflows, code review, log analysis, TDD\"\n  default_prompt: \"Use agent007 to run a workflow or task. Start with agent007_workflow_list to see what's available.\"\n\n\
dependencies:\n  tools:\n    - type: \"mcp\"\n      value: \"agent007\"\n      description: \"agent007 MCP orchestration server\"\n      transport: \"stdio\"\n      command: \"{}\"\n      args: [\"serve\"]\n",
        cmd.replace('\\', "\\\\")
    );

    let mut wrote = 0usize;
    if write_file(
        &skill_md,
        CODEX_SKILL_MD,
        "~/.codex/skills/agent007/SKILL.md",
        force,
    )? {
        wrote += 1;
    }
    if write_file(
        &agent_yaml,
        &skill_agent_yaml,
        "~/.codex/skills/agent007/agents/openai.yaml",
        force,
    )? {
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
///   .rules        — native Zed rules file (project root)
/// Project-local: .zed/ — Global: ~/.config/zed/
fn register_zed(zed_dir: &Path, project_dir: &Path, global: bool, force: bool) -> Result<()> {
    if let Some(parent) = zed_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(zed_dir)?;

    register_zed_settings(zed_dir, force)?;
    register_zed_tasks(zed_dir, force)?;
    if !global {
        register_zed_rules(project_dir, force)?;
    }
    Ok(())
}

fn register_zed_settings(zed_dir: &Path, force: bool) -> Result<()> {
    let settings_path = zed_dir.join("settings.json");

    let mut root = load_json_root(&settings_path, "Zed settings.json")?;

    let binary_path = which_agent007();
    let obj = root.as_object_mut().unwrap();

    // ── LSP ────────────────────────────────────────────────────────────────
    let lsp = obj
        .entry("lsp")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if lsp.contains_key("agent007") && !force {
        ok(&format!(
            "LSP already registered in {}",
            settings_path.display()
        ));
    } else {
        lsp.insert(
            "agent007".to_string(),
            serde_json::json!({
                "binary": {
                    "path": binary_path,
                    "arguments": ["serve-lsp", "--stdio"]
                }
            }),
        );
        ok(&format!(
            "Wrote lsp.agent007 → {binary_path} serve-lsp --stdio"
        ));
    }

    // ── MCP context_server ─────────────────────────────────────────────────
    let ctx = obj
        .entry("context_servers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();

    if ctx.contains_key("agent007") && !force {
        ok(&format!(
            "MCP context_server already registered in {}",
            settings_path.display()
        ));
    } else {
        ctx.insert(
            "agent007".to_string(),
            serde_json::json!({
                "command": binary_path,
                "args": ["serve"],
                "env": {}
            }),
        );
        ok(&format!(
            "Wrote context_servers.agent007 → {binary_path} serve"
        ));
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
        tools.insert(
            "mcp:agent007".to_string(),
            serde_json::json!({
                "default": "allow"
            }),
        );
        ok("Wrote agent.tool_permissions.mcp:agent007 → allow");
    }

    write_json_root(&settings_path, &root, "Zed settings.json")?;
    println!();
    warn("Restart Zed to activate the LSP and MCP server");
    Ok(())
}

fn register_zed_rules(project_dir: &Path, force: bool) -> Result<()> {
    let canonical_target = ensure_canonical_instruction_files(project_dir, force)?;
    let rules_path = project_dir.join(".rules");
    let content = zed_rules_md(&canonical_target);
    write_file(&rules_path, &content, ".rules", force)?;
    info("Zed auto-loads .rules into every Agent Panel interaction");
    Ok(())
}

fn ensure_canonical_instruction_files(project_dir: &Path, force: bool) -> Result<String> {
    let agents_path = project_dir.join("AGENTS.md");
    let shared_path = project_dir.join("AGENT007.md");
    let generated_path = project_dir.join("AGENTS.agent007.generated.md");

    write_file(
        &shared_path,
        AGENT007_SHARED_MD,
        "AGENT007.md",
        force || !shared_path.exists(),
    )?;

    if agents_path.exists() {
        if force {
            write_file(
                &generated_path,
                AGENT007_AGENTS_MD,
                "AGENTS.agent007.generated.md",
                true,
            )?;
            ok(&format!(
                "Preserved existing AGENTS.md → {}",
                agents_path.display()
            ));
            info(&format!(
                "Refreshed generated agent007 guidance → {}",
                generated_path.display()
            ));
        } else {
            write_file(
                &generated_path,
                AGENT007_AGENTS_MD,
                "AGENTS.agent007.generated.md",
                false,
            )?;
            ok(&format!(
                "AGENTS.md already exists — preserved ({})",
                agents_path.display()
            ));
            info("Generated agent007 companion guidance without overwriting AGENTS.md");
        }
        return Ok("AGENT007.md".to_string());
    }

    write_file(&agents_path, AGENT007_AGENTS_MD, "AGENTS.md", true)?;
    Ok("AGENT007.md".to_string())
}

fn register_claude_project_files(project_dir: &Path, claude_dir: &Path, force: bool) -> Result<()> {
    let canonical_target = ensure_canonical_instruction_files(project_dir, force)?;
    let claude_md_path = project_dir.join("CLAUDE.md");
    let claude_md = claude_md_wrapper(&canonical_target);
    write_file(&claude_md_path, &claude_md, "CLAUDE.md", force)?;

    let agents_dir = claude_dir.join("agents");
    let created_agents_dir = !agents_dir.exists();
    std::fs::create_dir_all(&agents_dir)?;
    if created_agents_dir {
        ok("agents/ created");
    }
    for (filename, content) in claude_agent_specs() {
        write_file(
            &agents_dir.join(filename),
            content,
            &format!("agents/{filename}"),
            force,
        )?;
    }
    Ok(())
}

fn register_cursor_project_files(project_dir: &Path, force: bool) -> Result<()> {
    let canonical_target = ensure_canonical_instruction_files(project_dir, force)?;
    let rules_dir = project_dir.join(".cursor").join("rules");
    std::fs::create_dir_all(&rules_dir)?;
    let content = cursor_rule_mdc(&canonical_target);
    write_file(
        &rules_dir.join("agent007.mdc"),
        &content,
        ".cursor/rules/agent007.mdc",
        force,
    )?;
    Ok(())
}

fn register_copilot_project_files(project_dir: &Path, force: bool) -> Result<()> {
    let canonical_target = ensure_canonical_instruction_files(project_dir, force)?;
    let github_dir = project_dir.join(".github");
    std::fs::create_dir_all(&github_dir)?;
    let content = copilot_instructions_md(&canonical_target);
    write_file(
        &github_dir.join("copilot-instructions.md"),
        &content,
        ".github/copilot-instructions.md",
        force,
    )?;
    Ok(())
}

fn register_codex_project_files(project_dir: &Path, force: bool) -> Result<()> {
    let canonical_target = ensure_canonical_instruction_files(project_dir, force)?;
    let codex_note_path = project_dir.join("AGENT007-CODEX.md");
    let content = codex_project_note_md(&canonical_target);
    write_file(&codex_note_path, &content, "AGENT007-CODEX.md", force)?;
    Ok(())
}

fn register_zed_tasks(zed_dir: &Path, force: bool) -> Result<()> {
    let tasks_path = zed_dir.join("tasks.json");

    let binary_path = which_agent007();
    let built_in_tasks = serde_json::json!([
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
    let mut tasks = if tasks_path.exists() {
        let raw = std::fs::read_to_string(&tasks_path)
            .with_context(|| format!("failed to read {}", tasks_path.display()))?;
        serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| {
            anyhow!(
                "failed to parse Zed tasks.json at {}: {error}. Refusing to overwrite an existing file.",
                tasks_path.display()
            )
        })?
    } else {
        serde_json::json!([])
    };
    let array = tasks.as_array_mut().ok_or_else(|| {
        anyhow!(
            "Zed tasks.json at {} must be a JSON array. Refusing to overwrite an existing file.",
            tasks_path.display()
        )
    })?;
    for task in built_in_tasks.as_array().unwrap() {
        let label = task
            .get("label")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("built-in Zed task is missing a label"))?;
        if let Some(index) = array.iter().position(|entry| {
            entry
                .get("label")
                .and_then(|value| value.as_str())
                .map(|value| value == label)
                .unwrap_or(false)
        }) {
            if force {
                array[index] = task.clone();
            }
        } else {
            array.push(task.clone());
        }
    }
    write_json_root(&tasks_path, &tasks, "Zed tasks.json")?;
    info("Run tasks via: Zed command palette → 'task: spawn'");
    Ok(())
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Seed the global ~/.agent007/ with built-in workflows and personas if the
/// directories are missing or empty. Called during project-local `init` so
/// every new project gets the globals even without `agent007 init --global`.
fn seed_global_if_missing(global_home: &Path) -> Result<()> {
    let skills_dir = global_home.join("skills");
    let skills_missing = !skills_dir.exists()
        || std::fs::read_dir(&skills_dir)
            .map(|d| d.flatten().count() == 0)
            .unwrap_or(true);
    if skills_missing {
        std::fs::create_dir_all(&skills_dir)?;
        let mut count = 0usize;
        for (filename, content) in crate::built_in_skills::ALL_SKILLS {
            if write_if_missing(
                &skills_dir.join(filename),
                content,
                &format!("~/.agent007/skills/{filename}"),
            )? {
                count += 1;
            }
        }
        if count > 0 {
            ok(&format!(
                "{count} built-in skills seeded to ~/.agent007/skills/"
            ));
        }
    }

    let wf_dir = global_home.join("workflows");
    let wf_missing = !wf_dir.exists()
        || std::fs::read_dir(&wf_dir)
            .map(|d| d.flatten().count() == 0)
            .unwrap_or(true);
    if wf_missing {
        std::fs::create_dir_all(&wf_dir)?;
        write_workflow_if_missing(
            &wf_dir.join("log-analysis.yaml"),
            WORKFLOW_LOG_ANALYSIS,
            "~/.agent007/workflows/log-analysis.yaml",
        )?;
        write_workflow_if_missing(
            &wf_dir.join("code-review.yaml"),
            WORKFLOW_CODE_REVIEW,
            "~/.agent007/workflows/code-review.yaml",
        )?;
        write_workflow_if_missing(
            &wf_dir.join("security-audit.yaml"),
            WORKFLOW_SECURITY_AUDIT,
            "~/.agent007/workflows/security-audit.yaml",
        )?;
        write_workflow_if_missing(
            &wf_dir.join("sparc.yaml"),
            WORKFLOW_SPARC,
            "~/.agent007/workflows/sparc.yaml",
        )?;
        write_workflow_if_missing(
            &wf_dir.join("tdd.yaml"),
            WORKFLOW_TDD,
            "~/.agent007/workflows/tdd.yaml",
        )?;
        write_workflow_if_missing(
            &wf_dir.join("ideation.yaml"),
            WORKFLOW_IDEATION,
            "~/.agent007/workflows/ideation.yaml",
        )?;
        write_workflow_if_missing(
            &wf_dir.join("feature.yaml"),
            WORKFLOW_FEATURE,
            "~/.agent007/workflows/feature.yaml",
        )?;
        write_workflow_if_missing(
            &wf_dir.join("brainstorm.yaml"),
            WORKFLOW_BRAINSTORM,
            "~/.agent007/workflows/brainstorm.yaml",
        )?;
        write_workflow_if_missing(
            &wf_dir.join("llm-council.yaml"),
            WORKFLOW_LLM_COUNCIL,
            "~/.agent007/workflows/llm-council.yaml",
        )?;
        ok("9 built-in workflows seeded to ~/.agent007/workflows/");
    }

    let personas_dir = global_home.join("personas");
    let personas_missing = !personas_dir.exists()
        || std::fs::read_dir(&personas_dir)
            .map(|d| d.flatten().count() == 0)
            .unwrap_or(true);
    if personas_missing {
        std::fs::create_dir_all(&personas_dir)?;
        let registry = agent007_personas::PersonaRegistry::built_in();
        let personas = {
            use agent007_core::PersonaProvider;
            registry.list()
        };
        let mut count = 0usize;
        for spec in &personas {
            let filename = spec
                .name
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
            let tools_str = spec
                .allowed_tools
                .iter()
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
            if write_if_missing(
                &path,
                &content,
                &format!("~/.agent007/personas/{filename}.toml"),
            )? {
                count += 1;
            }
        }
        if count > 0 {
            ok(&format!("{count} personas seeded to ~/.agent007/personas/"));
        }
    }
    Ok(())
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
default_model = "claude-sonnet-5"

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

const WORKFLOW_LLM_COUNCIL: &str = include_str!("../../workflows/llm-council.yaml");

const WORKFLOW_LOG_ANALYSIS: &str = r#"name: log-analysis
description: >
  Parallel log analysis team. Specialists run concurrently; the synthesizer
  aggregates all findings into a final report.
reliability:
  enabled: true
  recovery:
    enabled: true
    max_step_retries: 2
  budget_governor:
    enabled: true
    max_degradations_per_run: 1
    degrade_output_chars: 12000
  confidence:
    enabled: true
    low_terms:
      - "unsure"
      - "uncertain"
      - "not sure"
    missing_requires_approval: true
eval_gate:
  enabled: true
  release_class: false
  mode: fail-open
  baseline_window: 20
  min_baseline_runs: 3
  thresholds:
    max_quality_score_drop: 0.15
    max_cost_usd_increase: 2.0
    max_latency_ms_increase: 2500
    max_retry_increase: 1.0

steps:
  - id: find-errors
    agent: DebugAgent
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
reliability:
  enabled: true
  recovery:
    enabled: true
    max_step_retries: 2
  budget_governor:
    enabled: true
    max_degradations_per_run: 1
    degrade_output_chars: 12000
  confidence:
    enabled: true
    low_terms:
      - "unsure"
      - "uncertain"
      - "not sure"
    missing_requires_approval: true
eval_gate:
  enabled: true
  release_class: true
  mode: fail-closed
  baseline_window: 20
  min_baseline_runs: 3
  thresholds:
    max_quality_score_drop: 0.1
    max_cost_usd_increase: 2.0
    max_latency_ms_increase: 2500
    max_retry_increase: 1.0

steps:
  - id: security-review
    agent: SecurityReviewer
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are a security code reviewer. Review the following code for:
      - Injection vulnerabilities (SQL, command, XSS)
      - Authentication/authorization flaws
      - Insecure data handling (secrets, PII)
      - Dependency vulnerabilities

      Code / task: {{task}}

      ---
      Project architecture (from memory — use to avoid re-reading known files):
      {{memory.repo_brain}}

      Prior findings (do NOT repeat already-known issues):
      {{rag_context}}
    output: security_findings
    depends_on: []

  - id: performance-review
    agent: PerformanceEngineer
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are a performance code reviewer. Review the following code for:
      - Algorithmic complexity issues (O(n²), N+1 queries)
      - Memory leaks or excessive allocations
      - Blocking calls in async contexts
      - Missing indexes or caching opportunities

      Code / task: {{task}}

      ---
      Project architecture (from memory):
      {{memory.repo_brain}}

      Prior findings (do NOT repeat already-known issues):
      {{rag_context}}
    output: performance_findings
    depends_on: []

  - id: quality-review
    agent: CodeReviewer
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are a code quality reviewer. Review for:
      - Code smells and anti-patterns
      - Missing error handling
      - Test coverage gaps
      - Readability and maintainability issues

      Code / task: {{task}}

      ---
      Project notes and decisions:
      {{memory.project}}

      Prior findings (do NOT repeat already-known issues):
      {{rag_context}}
    output: quality_findings
    depends_on: []

  - id: synthesize
    agent: CodeReviewer
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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

const WORKFLOW_SECURITY_AUDIT: &str = r#"name: security-audit
description: >
  Deep security audit pipeline. OWASP, secrets, threat model, and dependency
  scanners run in parallel; the lead synthesizes a severity-ranked report.
reliability:
  enabled: true
  recovery:
    enabled: true
    max_step_retries: 2
  budget_governor:
    enabled: true
    max_degradations_per_run: 1
    degrade_output_chars: 12000
  confidence:
    enabled: true
    low_terms:
      - "unsure"
      - "uncertain"
      - "not sure"
    missing_requires_approval: true
eval_gate:
  enabled: true
  release_class: true
  mode: fail-closed
  baseline_window: 20
  min_baseline_runs: 3
  thresholds:
    max_quality_score_drop: 0.1
    max_cost_usd_increase: 3.0
    max_latency_ms_increase: 3000
    max_retry_increase: 1.0

steps:
  - id: owasp-scan
    agent: SecurityReviewer
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are an application security expert. Perform an OWASP Top 10 audit.

      Target: {{task}}

      Check:
      - A01 Broken Access Control (IDOR, path traversal, privilege escalation)
      - A02 Cryptographic Failures (weak ciphers, missing TLS, key exposure)
      - A03 Injection (SQL, command, template, XPath)
      - A04 Insecure Design (missing rate limits, absent threat controls)
      - A05 Security Misconfiguration (debug endpoints, default creds, verbose errors)
      - A07 Auth Failures (weak session, missing MFA, insecure password reset)
      - A08 Integrity Failures (unsafe deserialization, unsigned updates)
      - A09 Logging Failures (missing audit trail, sensitive data in logs)
      - A10 SSRF (internal service access from user-controlled URLs)

      Output a findings table: | Severity | OWASP ID | Finding | Location | Fix |

      ---
      Project architecture (use to focus on relevant attack surfaces):
      {{memory.repo_brain}}

      Prior security findings (skip already-known issues):
      {{rag_context}}
    output: owasp_findings
    depends_on: []

  - id: secrets-scan
    agent: SecurityReviewer
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are a secrets detection specialist. Scan the following for credential leaks.

      Target: {{task}}

      Find:
      - Hardcoded API keys, tokens, passwords, connection strings
      - Private keys or certificates in source/config
      - Credentials in environment variable defaults
      - Secrets in comments or test fixtures
      - Insecure secret storage patterns (plaintext files, unencrypted env)

      Output: | Severity | Type | Location | Pattern Found | Remediation |

      ---
      Project notes:
      {{memory.project}}
    output: secrets_findings
    depends_on: []

  - id: threat-model
    agent: Architect
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are a threat modeling expert using the STRIDE framework.

      Target: {{task}}

      For each trust boundary and component, analyze:
      | Threat | Component | Attack Vector | Impact | Mitigations |
      |--------|-----------|---------------|--------|-------------|
      | Spoofing | | | | |
      | Tampering | | | | |
      | Repudiation | | | | |
      | Information Disclosure | | | | |
      | Denial of Service | | | | |
      | Elevation of Privilege | | | | |

      Include: data flow diagram description, trust boundaries, attack surface summary.

      ---
      Architecture context:
      {{memory.repo_brain}}
    output: threat_model
    depends_on: []

  - id: dep-scan
    agent: SecurityReviewer
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are a supply chain security expert. Analyze dependencies for risk.

      Target: {{task}}

      Check:
      - Outdated packages with known CVEs (check Cargo.toml, package.json, requirements.txt)
      - Wildcard/unpinned version ranges that allow malicious upgrades
      - Packages with excessive permissions or unusual install scripts
      - Transitive dependency risks
      - License compliance issues (GPL contamination)

      Output: | Severity | Package | Current Version | CVE / Risk | Recommended Action |

      ---
      Project notes:
      {{memory.project}}
    output: dep_findings
    depends_on: []

  - id: synthesize
    agent: SecurityReviewer
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      Synthesize all security audit findings into a final executive report.

      OWASP FINDINGS:
      {{owasp_findings}}

      SECRETS SCAN:
      {{secrets_findings}}

      THREAT MODEL:
      {{threat_model}}

      DEPENDENCY SCAN:
      {{dep_findings}}

      Produce:
      1. **Executive Summary** (2-3 sentences, overall security posture)
      2. **Severity-ranked master findings table**: | Severity | Category | Finding | Location | Effort | Fix |
      3. **Attack surface map**: entry points, trust boundaries, data flows
      4. **Top 5 Priority Remediations** with code examples
      5. **Security Score**: 0–100 with breakdown by category
    output: security_report
    depends_on: [owasp-scan, secrets-scan, threat-model, dep-scan]
"#;

const WORKFLOW_SPARC: &str = r#"name: sparc
description: >
  SPARC methodology pipeline: Spec → Pseudocode → Architecture → Refinement → Completion.
  Each phase feeds into the next.
reliability:
  enabled: true
  recovery:
    enabled: true
    max_step_retries: 2
  budget_governor:
    enabled: true
    max_degradations_per_run: 1
    degrade_output_chars: 12000
  confidence:
    enabled: true
    low_terms:
      - "unsure"
      - "uncertain"
      - "not sure"
    missing_requires_approval: true
eval_gate:
  enabled: true
  release_class: true
  mode: fail-closed
  baseline_window: 20
  min_baseline_runs: 3
  thresholds:
    max_quality_score_drop: 0.12
    max_cost_usd_increase: 3.0
    max_latency_ms_increase: 3000
    max_retry_increase: 1.0

steps:
  - id: spec
    agent: Researcher
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      SPARC Phase 1 — Specification.
      Write a detailed specification for: {{task}}
      Include: goals, constraints, user stories, acceptance criteria, edge cases.

      ---
      Project context (use to understand existing conventions and avoid duplicate work):
      {{memory.repo_brain}}

      Project decisions and notes:
      {{memory.project}}
    output: specification
    depends_on: []

  - id: pseudocode
    agent: Coder
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      SPARC Phase 2 — Pseudocode.
      Based on this specification:
      {{specification}}

      Write structured pseudocode with logic flow, data structures, and algorithms.
    output: pseudocode
    depends_on: [spec]

  - id: architecture
    agent: Architect
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      SPARC Phase 3 — Architecture.
      Spec: {{specification}}
      Pseudocode: {{pseudocode}}

      Design the system architecture: components, interfaces, data flow, dependencies.

      ---
      Project architecture context:
      {{memory.repo_brain}}
    output: architecture
    depends_on: [pseudocode]

  - id: refinement
    agent: CodeReviewer
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      SPARC Phase 4 — Refinement.
      Review the architecture for correctness, security, performance, and scalability.
      Architecture: {{architecture}}
      Identify and fix issues.
    output: refined_design
    depends_on: [architecture]

  - id: completion
    agent: ExpertCoder
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
reliability:
  enabled: true
  recovery:
    enabled: true
    max_step_retries: 2
  budget_governor:
    enabled: true
    max_degradations_per_run: 1
    degrade_output_chars: 10000
  confidence:
    enabled: true
    low_terms:
      - "unsure"
      - "uncertain"
      - "not sure"
    missing_requires_approval: true
eval_gate:
  enabled: true
  release_class: true
  mode: fail-closed
  baseline_window: 20
  min_baseline_runs: 3
  thresholds:
    max_quality_score_drop: 0.12
    max_cost_usd_increase: 2.0
    max_latency_ms_increase: 2000
    max_retry_increase: 1.0

steps:
  - id: red
    agent: TestDesigner
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      TDD Red Phase — write a failing test for: {{task}}
      Produce: test file with failing test cases covering the requirement.
      Tests must fail because the implementation doesn't exist yet.

      ---
      Project context (use to match existing test patterns and conventions):
      {{memory.repo_brain}}
    output: failing_tests
    depends_on: []

  - id: green
    agent: Coder
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      TDD Green Phase — write minimal code to make these tests pass:
      {{failing_tests}}

      Requirement: {{task}}
      Write the simplest possible implementation. Do not over-engineer.
    output: implementation
    depends_on: [red]

  - id: blue
    agent: ExpertCoder
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
reliability:
  enabled: true
  recovery:
    enabled: true
    max_step_retries: 2
  budget_governor:
    enabled: true
    max_degradations_per_run: 1
    degrade_output_chars: 12000
  confidence:
    enabled: true
    low_terms:
      - "unsure"
      - "uncertain"
      - "not sure"
    missing_requires_approval: true
eval_gate:
  enabled: true
  release_class: true
  mode: fail-closed
  baseline_window: 30
  min_baseline_runs: 3
  thresholds:
    max_quality_score_drop: 0.1
    max_cost_usd_increase: 3.0
    max_latency_ms_increase: 3000
    max_retry_increase: 1.0

steps:
  - id: research
    agent: Researcher
    model: claude-haiku-4-5-20251001
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are a research specialist. Gather comprehensive context for:

      {{task}}

      Produce:
      1. Background and prior art
      2. Relevant patterns, technologies, and approaches
      3. Key constraints and risks
      4. Open questions that need human input
      5. Recommended direction with rationale

      Be thorough — this output drives all downstream steps.

      ---
      Project context (use to understand existing architecture before suggesting approaches):
      {{memory.repo_brain}}

      Project decisions and notes:
      {{memory.project}}
    output: research_output
    depends_on: []

  - id: review-research
    agent: Researcher
    model: claude-sonnet-5
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
    model: claude-sonnet-5
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
    model: claude-opus-5
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
    model: claude-sonnet-5
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
    model: claude-sonnet-5
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
reliability:
  enabled: true
  recovery:
    enabled: true
    max_step_retries: 2
  budget_governor:
    enabled: true
    max_degradations_per_run: 1
    degrade_output_chars: 12000
  confidence:
    enabled: true
    low_terms:
      - "unsure"
      - "uncertain"
      - "not sure"
    missing_requires_approval: true
eval_gate:
  enabled: true
  release_class: true
  mode: fail-closed
  baseline_window: 30
  min_baseline_runs: 3
  thresholds:
    max_quality_score_drop: 0.1
    max_cost_usd_increase: 3.0
    max_latency_ms_increase: 3000
    max_retry_increase: 1.0

steps:
  - id: load-context
    agent: Researcher
    model: claude-haiku-4-5-20251001
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
    model: claude-sonnet-5
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
    model: claude-opus-5
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
    model: claude-sonnet-5
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
      {{WORKFLOW_OPERATING_PROTOCOL}}

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
    model: claude-sonnet-5
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

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

const WORKFLOW_BRAINSTORM: &str = r#"name: brainstorm
description: >
  Lightweight brainstorm-to-docs pipeline. Free-form ideation → human direction approval →
  PRD + ideation document written to docs/. Stops before architecture and milestones.
  Use this to capture ideas and produce a PRD before committing to the full ideation workflow.
  The generated docs serve as direct input to /agent007-workflow-ideation or /dev-architect.
reliability:
  enabled: true
  recovery:
    enabled: true
    max_step_retries: 2
  budget_governor:
    enabled: true
    max_degradations_per_run: 1
    degrade_output_chars: 12000
  confidence:
    enabled: true
    low_terms:
      - "unsure"
      - "uncertain"
      - "not sure"
    missing_requires_approval: true
eval_gate:
  enabled: true
  release_class: false
  mode: fail-open
  baseline_window: 20
  min_baseline_runs: 3
  thresholds:
    max_quality_score_drop: 0.15
    max_cost_usd_increase: 2.0
    max_latency_ms_increase: 2500
    max_retry_increase: 1.0

steps:
  - id: brainstorm
    agent: Researcher
    model: claude-sonnet-5
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are a brainstorming specialist using the Double Diamond design-thinking methodology.
      Explore the problem space for:

      {{task}}

      Phase 1 — Discover: What is the pain? Who has it? What is the current workaround or status quo?
      Phase 2 — Define: Frame the problem precisely. What constraints cannot be compromised?
      Phase 3 — Develop: Generate 3–5 meaningfully different approaches. Make each a real alternative,
        not just a variation. Include at least one unconventional option.
      Phase 4 — Converge: Recommend one direction with clear rationale.

      For each approach provide:
      - Name and one-sentence summary
      - How it works (2-3 sentences)
      - Strengths
      - Weaknesses / risks
      - Effort estimate (Low / Medium / High)

      End with:
      - Recommended approach and rationale
      - Key risks to mitigate
      - Open questions that need human input
      - Assumptions to validate before building

      ---
      Project context (understand existing architecture before suggesting approaches):
      {{memory.repo_brain}}

      Project decisions and notes:
      {{memory.project}}
    output: brainstorm_output
    depends_on: []

  - id: review-direction
    agent: Researcher
    model: claude-sonnet-5
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      Brainstorm complete. Human: please review the approaches and select a direction.

      BRAINSTORM OUTPUT:
      {{brainstorm_output}}

      Present a concise summary of each option and the key trade-offs. Flag the open questions
      that need human input. The human can approve one option, request a different direction,
      or ask for more exploration on a specific approach.
    output: direction_approved
    depends_on: [brainstorm]

  - id: write-prd
    agent: Planner
    model: claude-sonnet-5
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      You are a product manager. Write a Product Requirements Document (PRD) for:

      {{task}}

      Based on:
      BRAINSTORM: {{brainstorm_output}}
      APPROVED DIRECTION: {{direction_approved}}

      Include:
      - Executive summary and goals
      - User stories with acceptance criteria (As a... I want... So that...)
      - Functional requirements
      - Non-functional requirements (performance, security, reliability)
      - Out-of-scope items
      - Success metrics
      - Open questions and assumptions
    output: prd_doc
    depends_on: [review-direction]

  - id: write-ideation-doc
    agent: DocumentationWriter
    model: claude-haiku-4-5-20251001
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      Capture the brainstorm and approved direction as a structured Ideation Document.

      TASK: {{task}}
      BRAINSTORM: {{brainstorm_output}}
      APPROVED DIRECTION: {{direction_approved}}

      Write a document covering:
      - Problem statement and context
      - Goals and non-goals
      - Constraints
      - Options explored (summary of each approach from the brainstorm)
      - Selected direction and rationale
      - Open questions
      - Next steps
    output: ideation_doc
    depends_on: [review-direction]

  - id: write-docs
    agent: DocumentationWriter
    model: claude-haiku-4-5-20251001
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      Write the brainstorm outputs as project documentation files.

      Use the file_write tool to create the following files
      (create the docs/ directory if it does not exist):

      1. docs/ideation.md
         Content: {{ideation_doc}}

      2. docs/prd.md
         Content: {{prd_doc}}

      After writing both files, produce a brief summary listing the files
      created and their purpose. These files should be committed to version
      control and serve as input to the architecture phase.
    output: docs_written
    depends_on: [write-prd, write-ideation-doc]

  - id: present-summary
    agent: Planner
    model: claude-sonnet-5
    requires_approval: true
    prompt: |
      {{WORKFLOW_OPERATING_PROTOCOL}}

      The brainstorm phase is complete. Present the results for final review.

      ## Brainstorm Summary

      ### Topic
      {{task}}

      ### Approved Direction
      {{direction_approved}}

      ### Documents Written
      - `docs/ideation.md` — full ideation document with all explored options
      - `docs/prd.md` — product requirements document

      Status: {{docs_written}}

      ### Next Steps
      To continue to full architecture and project planning:
        /agent007-workflow-ideation {{task}}

      To go directly to architecture:
        /dev-architect (use docs/prd.md as input)

      To start building a specific feature:
        /agent007-workflow-feature <feature name>

      ---
      Human: please review the brainstorm summary above.
      Approve to proceed to architecture, or edit to refine the direction.
    output: final_approval
    depends_on: [write-docs]
"#;

// ── Host instruction templates and Claude Code sub-agent definitions ──────

const AGENT007_SHARED_MD: &str = r#"# agent007 — AI Orchestration Rules

You have access to the **agent007** MCP server.

**Rule #1:** For any non-trivial task, route through agent007 before doing broad
free-form reasoning, broad file reads, or ad-hoc shell/Python parsing.

agent007 is the control plane. Use it for workflows, personas, repo intelligence,
memory, and deterministic extraction.

## Core cycle

```text
1. TASK
   -> user asks for work

2. CONTROL
   -> use agent007_run / agent007_skill_run / agent007_workflow_run
   -> get a run, prompt, or structured plan

3. WORK
   -> execute with normal editor tools
   -> read files, edit code, run commands, inspect diffs

4. RECORD
   -> when hosted flows ask for it, call agent007_record_tokens
   -> preserve useful output in memory and dashboard telemetry

5. LEARN
   -> future runs reuse repo brain, memory, and prior outputs
```

The important rule:

```text
for multi-step or high-context work, route through agent007 first
```

The durable file/log inspection rule:

```text
for repository files, logs, CSV/JSON reports, diffs, workflow state, metrics,
and code search, use agent007 ETR tools first; do not silently fall back to
ad-hoc shell/Python readers in a new, resumed, or forked thread.
```

## Coordinator model

Top-level coordinators:
- **Architect** — design, decomposition, delivery planning, workflow choice
- **Analyst** — investigation, review, root cause analysis, evidence synthesis

Specialist workers underneath them include:
- **Planner**
- **Coder** / **ExpertCoder**
- **CodeReviewer**
- **SecurityReviewer**
- **PerformanceEngineer**
- **DocumentationWriter** / **DocsManager**
- **UIUXDesigner**
- **DebugAgent**
- **Researcher**
- **DevOpsEngineer**

Keep the top-level small. Prefer richer workers underneath rather than adding many peer coordinators.

## Routing ladder

1. **Quick single-step work** → `agent007_run`
2. **Repeatable expert pattern** → `agent007_skill_run`
3. **Multi-step / approval-gated / parallel work** → `agent007_workflow_run`
4. **Exact extraction or structured analysis** → `agent007_etr_list` then `agent007_etr_call`
5. **Execution of builds/tests/scripts** → shell/build/test tools are fine

## Core tools

| Tool | Purpose |
|------|---------|
| `agent007_run` | Run a quick task through the agent stack |
| `agent007_skill_list` | Discover installed skills |
| `agent007_skill_run` | Run a named skill by trigger |
| `agent007_workflow_list` | List available workflows |
| `agent007_workflow_run` | Run a workflow synchronously |
| `agent007_workflow_start` / `agent007_workflow_next` | Drive hosted workflow sessions |
| `agent007_workflow_submit_step` / `agent007_workflow_approve` | Submit hosted step output and approval decisions |
| `agent007_record_tokens` | Persist hosted output and dashboard metrics |
| `agent007_context_compile` | Pull repo brain + memory + relevant files |
| `agent007_memory_read` / `agent007_memory_write` | Read or persist high-signal context |
| `agent007_run_history` | Review prior runs |
| `agent007_repo_brain_refresh` | Rebuild project summary memory |
| `agent007_etr_list` | Discover available ETR tools and schemas |
| `agent007_etr_call` | Read/search/slice/analyze files through ETR |

## Skills to prefer

- `/dev-architect`
- `/dev-debug`
- `/dev-pr-review`
- `/dev-tdd`
- `/code-refactor`
- `/code-optimize`
- `/code-security-audit`
- `/code-test-gen`
- `/code-document`
- `/meta-analyze-codebase`
- `/project-prd`
- `/project-plan`
- `/project-release`
- `/project-changelog`
- `/brainstorm`
- `/frontend-designer`

## Workflows to prefer

- `feature`
- `code-review`
- `security-audit`
- `sparc`
- `tdd`
- `ideation`
- `brainstorm`
- `log-analysis`

## Context and memory

Before broad edits on an unfamiliar codebase:

```text
agent007_context_compile(task="<what you are about to do>")
```

Persist high-signal findings:

```text
agent007_memory_write(scope="project", key="<topic>", value="<finding>")
```

Review prior work:

```text
agent007_run_history()
```

## Execution vs interpretation

Use shell/build/test tools for **execution**:
- `cargo test`
- `cargo build`
- `npm run build`
- `pytest`
- existing repo scripts

Use agent007 / ETR for **interpretation and extraction**:
- summarize build warnings
- inspect JSON or tables
- query workflow state
- compare artifacts or metrics
- grep/glob/status tasks
- repo graph callers/callees/context

Do **not** default to custom Python or shell parsing if ETR can do the job.
If shell or Python is used for reading/analyzing files, state why ETR was not
sufficient. Python is acceptable for deterministic bulk edits when `apply_patch`
would be impractical, but not as the default file reader.

## ETR-first rule

Prefer ETR built-ins first for deterministic tasks. Common choices:

| ETR tool | Use |
|----------|-----|
| `etr.artifact_read` | Read source files, reports, JSON, and text artifacts with guardrails |
| `etr.grep` | Search source/logs for code paths, symbols, errors, and report fields |
| `etr.glob` | Discover files under source, report, log, and artifact directories |
| `etr.file_stat` | Check existence, size, and timestamps |
| `etr.diff` | Compare text artifacts deterministically |
| `etr.csv_slice` | Inspect CSV rows with optional column selection |
| `etr.table_select` / `etr.table_stats` | Select, filter, and summarize CSV/JSONL tables |
| `etr.metrics_summary` / `etr.delta_compare` | Summarize metrics and compare baseline vs candidate |
| `etr.json_extract` / `etr.json_query` / `etr.json_query_v2` | Inspect JSON manifests, reports, and state files |
| `etr.text_extract` | Extract regex matches from text or files |
| `etr.logs_slice` / `etr.logs_correlate` | Slice and correlate logs |
| `etr.workflow_status_summary` / `etr.workflow_outputs_index` / `etr.workflow_step_health` | Inspect hosted workflow state compactly |
| `etr.graph_build` / `etr.graph_status` | Build or inspect repo graph state |
| `etr.symbol_lookup` / `etr.callers` / `etr.callees` | Query symbol-level structure |
| `etr.impact_radius` / `etr.context_bundle` | Build graph-aware edit or review context |

If unsure:

```text
agent007_etr_list()
agent007_etr_call(tool="etr.grep", input={"path":"<path>", "pattern":"<regex>", "context_lines":2})
```

## Anti-patterns

Avoid:
- writing throwaway Python to parse JSON/logs/tables when ETR exists
- silently falling back to shell/Python readers after an ETR miss without saying why
- broad-reading many files before trying `agent007_context_compile`
- improvising on a task that clearly fits a skill or workflow
- treating hosted-MCP mode as if the host should ignore agent007
- pasting raw shell output as the final answer when synthesis is needed

## Hosted-MCP reality

In hosted-MCP mode, the host LLM is still the planner. That means it must deliberately
choose agent007 surfaces:
- workflows for multi-step delivery
- skills for repeatable expert work
- personas for role-specific tasks
- ETR for deterministic extraction

In every new, resumed, or forked thread, re-apply the ETR-first rule before
touching logs, validation reports, generated artifacts, algorithm code, or UI export code.

## Examples

- **Build warning analysis:** run the build with shell, then use agent007 / ETR to interpret it
- **Large feature request:** route to `feature` or `sparc`, do not broad-plan it by hand
- **Code review or audit:** use `code-review` or `security-audit`
- **Repo structure question:** use Repo Intelligence / graph tools first, then read only pointed files
- **JSON/report question:** use `etr.json_query` or `etr.artifact_read`, not custom Python
- **CSV/metrics question:** use `etr.csv_slice`, `etr.table_stats`, or `etr.metrics_summary`
"#;

const AGENT007_AGENTS_MD: &str = r#"# agent007 — Codex entrypoint

Codex reads `AGENTS.md` automatically. The detailed shared agent007 operating
contract is `AGENT007.md`.

Before any non-trivial task, read and follow `AGENT007.md`. This `AGENTS.md`
file intentionally stays small so the shared contract can also be loaded by
Claude Code, Cursor, Zed, and Copilot without duplicating content.

Critical bootstrap rules:
- Route non-trivial or high-context work through agent007 first.
- Use `agent007_context_compile` before broad edits on unfamiliar code.
- Use `agent007_etr_list` and `agent007_etr_call` before custom shell/Python readers.
- Use shell for execution/build/test; use agent007/ETR for interpretation and extraction.
- If shell/Python is used as a reader/parser, explain why ETR was not sufficient.
- Do not add `--no-dashboard` to `agent007 serve`; the dashboard is served by default.
"#;

const CLAUDE_AGENT_ARCHITECT: &str = r#"---
name: agent007-architect
description: >
  Meta-orchestrator for the agent007 system. Routes any complex engineering task to the
  right workflow or skill — code review, feature delivery, TDD, ideation, log analysis,
  security audit — and synthesizes results into a clear, actionable report.
  Use this agent when a task needs multiple specialist perspectives or parallel execution.
---

You are the **agent007 Architect** — a senior engineering lead who routes and orchestrates
specialist AI agents. You never guess at answers; you delegate to the right tool and
synthesize the results into something immediately useful.

## Decision framework

Before acting, ask: *what kind of work is this?*

| Request type | Workflow to use |
|---|---|
| "Review this code / PR / diff" | `code-review` |
| "Find bugs, security issues, vulnerabilities" | `security-audit` |
| "Build this feature / implement X" | `feature` (full cycle) or `sparc` (greenfield) |
| "Write tests for X" / TDD approach | `tdd` |
| "Analyze these logs / errors / traces" | `log-analysis` |
| "I have an idea, help me think it through" | `brainstorm` |
| "Plan this project / write PRD / architect" | `ideation` |

For single-shot tasks that don't need a full workflow, use `agent007_task_submit` with the
appropriate persona from `agent007_persona_list`.

## Available skills (single-step, invoked with agent007_skill_run)

- `/brainstorm` — explore a problem space, generate 3–5 approaches with trade-offs
- `/dev-architect` — design system architecture from requirements
- `/dev-debug` — systematic hypothesis-driven debugging
- `/dev-pr-review` — thorough PR review with actionable feedback
- `/dev-tdd` — TDD red-green-refactor cycle
- `/code-refactor` — identify code smells, propose targeted improvements
- `/code-optimize` — profile analysis, performance optimization
- `/code-security-audit` — OWASP, secrets scan, threat modeling
- `/code-test-gen` — generate comprehensive test suites with edge cases
- `/code-document` — generate API docs, architecture docs, inline documentation
- `/meta-analyze-codebase` — analyze tech stack, patterns, architecture
- `/project-prd` — product requirements with user stories and constraints
- `/project-plan` — break features into tasks with estimates and dependencies
- `/project-changelog` — generate changelogs from git history
- `/project-release` — version strategy, release notes, rollback planning

## How to execute

1. **Identify the request** — understand what outcome the user needs
2. **Check workflows** — call `agent007_workflow_list` if you're unsure what's available
3. **Run the workflow** — `agent007_workflow_run` with `name` + `task` (full context in task)
4. **Present results** — structure your synthesis: summary → key findings → actions

## Output format

Always end with:
- **TL;DR** — one sentence
- **Top 3 actions** — specific, concrete, ordered by priority
- **Which workflow ran** and why you chose it

If a workflow produces a long report, summarize it and highlight only what needs immediate
attention. Never paste raw workflow output without synthesis.
"#;

const CLAUDE_AGENT_ANALYST: &str = r#"---
name: agent007-analyst
description: >
  Deep analysis specialist for code, logs, and systems. Runs the appropriate workflow
  (log-analysis, code-review, or security-audit) and delivers a severity-ranked report
  with root causes, evidence, and prioritized fixes. Ideal for debugging incidents,
  auditing PRs, or understanding what's wrong with a system.
---

You are the **agent007 Analyst** — a specialist who digs deep, finds root causes, and
explains findings with precision. You don't skim; you investigate.

## What you handle

- **Log analysis** — application logs, crash reports, stack traces, access logs
- **Code review** — any language, any scope (single file to entire PR diff)
- **Security audit** — OWASP, secrets, dependency vulnerabilities, threat modeling
- **Incident investigation** — correlate signals across logs, metrics, and code

## Workflow routing

Pick the right workflow based on what you're given:

```
Logs / traces / error output  →  agent007_workflow_run(name="log-analysis", task=...)
Code / diff / PR              →  agent007_workflow_run(name="code-review",   task=...)
Security focus                →  agent007_workflow_run(name="security-audit", task=...)
```

Each workflow runs multiple specialist agents in parallel — you get coverage across error
patterns, performance, security, and style simultaneously.

## How to execute

1. **Triage the input** — identify what you're analyzing (logs, code, system description)
2. **Gather context** — if the user gave you a file path or vague description, ask for the
   actual content or use bash/read tools to fetch it before running the workflow
3. **Run the workflow** — pass the full content as the `task` parameter, not a summary
4. **Synthesize and present** results in the structure below

## Report structure

### 🔴 Critical Issues (P0) — fix immediately
List issues that cause data loss, security breaches, or complete outages.

### 🟠 High Priority (P1) — fix this sprint
Issues causing degraded functionality, performance problems, or moderate risk.

### 🟡 Medium Priority (P2) — address in backlog
Code quality, maintainability, minor inefficiencies.

### Root Cause Analysis
For each significant finding: what is it, where is it, why does it happen, what evidence
confirms it.

### Recommended Fixes
Concrete, copy-pasteable suggestions — not vague advice like "improve error handling".

### What to do next
Ordered list of 3–5 specific actions the user should take right now.

## Principles

- **Evidence over opinion** — cite line numbers, log timestamps, specific tokens
- **Signal over noise** — 5 real findings beat 20 style nitpicks
- **Root cause over symptoms** — trace errors to their source, not just where they manifest
- **Actionable over academic** — every finding needs a concrete next step
"#;

const CLAUDE_AGENT_BUILDER: &str = r#"---
name: agent007-builder
description: >
  Delivery specialist for implementation-heavy work. Uses feature, sparc, or tdd
  workflows and builder personas to turn plans into shippable code.
---

You are the **agent007 Builder** — an implementation-focused delivery agent.

Use this agent when:
- the user wants a feature built or refactored
- the task is implementation-first rather than analysis-first
- there is already enough clarity to start coding

Preferred routing:
1. multi-step implementation → `feature`, `sparc`, or `tdd`
2. focused implementation slice → `agent007_task_submit` with `Coder` or `ExpertCoder`
3. broad codebase edit → `agent007_context_compile` first

Use shell for build/test execution, but prefer ETR or repo graph tools for interpretation.
"#;

const CLAUDE_AGENT_REVIEWER: &str = r#"---
name: agent007-reviewer
description: >
  Quality gate specialist for correctness, maintainability, security, and test coverage.
  Uses code-review or security-audit workflows and synthesizes actionable findings.
---

You are the **agent007 Reviewer** — a quality and risk gate.

Use this agent when:
- the user asks for review, audit, critique, or validation
- a change set needs correctness, maintainability, or security feedback
- a task finished and now needs a quality pass

Routing:
- general review → `code-review`
- security-heavy review → `security-audit`
- focused direct review → `agent007_task_submit` with `CodeReviewer` or `SecurityReviewer`
"#;

const CLAUDE_AGENT_FRONTEND_DESIGNER: &str = r#"---
name: agent007-frontend-designer
description: >
  Frontend and UX specialist for dashboard/UI changes, interaction design,
  accessibility, and implementation-ready component direction.
---

You are the **agent007 Frontend Designer** — a UI delivery specialist.

Use this agent when:
- the task touches dashboard/UI/frontend flows
- interaction, accessibility, or visual hierarchy matters
- the user wants a frontend surface critiqued, redesigned, or implemented

Routing:
- focused frontend work → `/frontend-designer`
- direct persona routing → `agent007_task_submit` with `UIUXDesigner`
- broad surface → `agent007_context_compile` before editing
"#;

const CLAUDE_AGENT_RELEASE_MANAGER: &str = r#"---
name: agent007-release-manager
description: >
  Release and rollout specialist for versioning, changelogs, validation gates,
  release notes, and rollback planning.
---

You are the **agent007 Release Manager** — a controlled ship/no-ship operator.

Use this agent when:
- preparing a release
- deciding patch/minor release scope
- producing release notes or changelog summaries
- checking rollout or rollback readiness

Routing:
- release strategy → `/project-release`
- changelog generation → `/project-changelog`
- direct implementation-adjacent release prep → `agent007_task_submit` with `DevOpsEngineer`
"#;

const CLAUDE_AGENT_PLANNER: &str = r#"---
name: agent007-planner
description: >
  Planning specialist for milestones, execution slices, dependencies, and acceptance criteria.
---

You are the **agent007 Planner** — a planning and sequencing specialist.

Use this agent when:
- the work is underspecified
- a feature needs slicing into milestones
- you need explicit dependencies, validation, and acceptance criteria

Routing:
- `/project-plan`
- `/project-prd`
- direct persona `Planner`
"#;

fn claude_md_wrapper(canonical_target: &str) -> String {
    format!(
        r#"# Claude Code project guidance

@AGENTS.md
@{canonical_target}

## Claude Code

- Claude Code reads `CLAUDE.md`, not `AGENTS.md`; this file imports both the Codex entrypoint and the shared agent007 contract.
- Treat `{canonical_target}` as the detailed shared operating contract.
- For non-trivial work, route through agent007 before broad free-form reasoning.
- In hosted-MCP mode, Claude is still the planner, so it must deliberately choose agent007 workflows, skills, personas, and ETR tools.
- Use shell for execution; use agent007 / ETR for interpretation and structured extraction.
- Before custom shell/Python readers, use `agent007_etr_list` and `agent007_etr_call`; if you fall back, say why ETR was not sufficient.
"#
    )
}

fn cursor_rule_mdc(canonical_target: &str) -> String {
    format!(
        r#"---
description: agent007 routing and repo-intelligence rules
alwaysApply: true
---

@AGENTS.md
@{canonical_target}

- Prefer agent007 for non-trivial work.
- Prefer `agent007_etr_list` / `agent007_etr_call` for deterministic extraction and repo graph queries.
- Use shell/build/test tools for execution, not ad-hoc parsing; if you fall back to shell/Python readers, say why ETR was not sufficient.
"#
    )
}

fn copilot_instructions_md(canonical_target: &str) -> String {
    format!(
        r#"# agent007 guidance for GitHub Copilot

Also honor `AGENTS.md` as the Codex entrypoint and `{canonical_target}` as the primary shared agent007 operating contract for this repository.

- Route non-trivial tasks through agent007 first.
- Prefer skills and workflows before broad free-form planning.
- Use `agent007_etr_list` / `agent007_etr_call` for deterministic extraction/query/status tasks before custom shell/Python parsing.
- Use shell for execution of builds/tests/scripts, then synthesize results through agent007.
- If shell/Python is used as a reader/parser, explain why ETR was not sufficient.
"#
    )
}

fn zed_rules_md(canonical_target: &str) -> String {
    format!(
        r#"# agent007 rules for Zed

Use `{canonical_target}` as the detailed shared agent007 operating contract for this repository. `AGENTS.md` remains the Codex entrypoint.

- Prefer agent007 for non-trivial work.
- Prefer Repo Intelligence and `agent007_etr_call` before broad file reads or ad-hoc parsing.
- Use workflows for multi-step delivery and skills for repeatable specialist work.
- If shell/Python is used as a reader/parser, explain why ETR was not sufficient.
"#
    )
}

fn codex_project_note_md(canonical_target: &str) -> String {
    format!(
        r#"# Codex + agent007 project note

Codex reads `AGENTS.md` automatically. `AGENTS.md` points to `{canonical_target}`, the detailed shared agent007 operating contract for this repository.

Use agent007 first for:
- multi-step features
- code review / security review
- repo intelligence / graph interrogation
- deterministic extraction via ETR

Use `agent007_etr_list` / `agent007_etr_call` before custom shell/Python readers.
Use shell for execution and verification, not as the default interpretation layer.
If shell/Python is used as a reader/parser, explain why ETR was not sufficient.
"#
    )
}

fn claude_agent_specs() -> [(&'static str, &'static str); 7] {
    [
        ("agent007-architect.md", CLAUDE_AGENT_ARCHITECT),
        ("agent007-analyst.md", CLAUDE_AGENT_ANALYST),
        ("agent007-planner.md", CLAUDE_AGENT_PLANNER),
        ("agent007-builder.md", CLAUDE_AGENT_BUILDER),
        ("agent007-reviewer.md", CLAUDE_AGENT_REVIEWER),
        (
            "agent007-frontend-designer.md",
            CLAUDE_AGENT_FRONTEND_DESIGNER,
        ),
        ("agent007-release-manager.md", CLAUDE_AGENT_RELEASE_MANAGER),
    ]
}

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
For any non-trivial coding task, route through `agent007_dispatch` first.

Top-level coordinators:
- `Architect` — design, decomposition, delivery planning
- `Analyst` — investigation, review, evidence synthesis

Specialist workers exist underneath them:
- `Planner`
- `Coder` / `ExpertCoder`
- `CodeReviewer`
- `SecurityReviewer`
- `PerformanceEngineer`
- `DocumentationWriter`
- `UIUXDesigner`
- `DebugAgent`
- `Researcher`

Use shell/build/test tools for **execution**.  
Use agent007 / ETR for **interpretation, extraction, status, and repo graph queries**.

## Simple Command Mode (Recommended in Codex)

When the user uses command-like text (for example `$agent007 ...`, `/agent007 ...`, or `@agent007 ...`),
prefer this single MCP tool:

```
mcp__agent007__agent007_dispatch
```

Dispatch examples:

```
$agent007 wf tdd add login rate limiting
$agent007 workflow code-review review current diff
$agent007 skill /brainstorm onboarding ideas
$agent007 /dev-pr-review review this patch
$agent007 run refactor auth module
```

`agent007_dispatch` is additive convenience only. Existing direct tools remain valid.

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
| Simple command-style routing | `mcp__agent007__agent007_dispatch` |
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
| `brainstorm` | Free-form ideation → PRD + ideation doc (lightweight) |
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_mcp_written_to_new_location_and_settings_migrated() {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().to_path_buf();
        let claude_dir = project_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        // Simulate old settings.json that has mcpServers.agent007 (pre-migration)
        let settings_path = claude_dir.join("settings.json");
        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&json!({
                "theme": "dark",
                "mcpServers": {
                    "agent007": {
                        "command": "/existing/agent007",
                        "args": ["serve"]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        register_claude_mcp("/new/agent007", &claude_dir, &project_dir, false, false).unwrap();

        // MCP entry should now be in .mcp.json at project root
        let mcp_path = project_dir.join(".mcp.json");
        let mcp_root: JsonValue =
            serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert_eq!(
            mcp_root["mcpServers"]["agent007"]["command"],
            "/new/agent007"
        );

        // settings.json should have theme preserved, mcpServers removed, statusLine added
        let settings_root: JsonValue =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(settings_root["theme"], "dark");
        assert!(
            settings_root.get("mcpServers").is_none()
                || settings_root["mcpServers"]["agent007"].is_null(),
            "old mcpServers.agent007 should be removed from settings.json"
        );
        assert_eq!(settings_root["statusLine"]["type"], "command");
    }

    #[test]
    fn zed_settings_invalid_json_is_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let zed_dir = temp.path().join(".zed");
        std::fs::create_dir_all(&zed_dir).unwrap();
        let settings_path = zed_dir.join("settings.json");
        let original = "{ invalid json";
        std::fs::write(&settings_path, original).unwrap();

        let err = register_zed_settings(&zed_dir, false).unwrap_err();
        assert!(err.to_string().contains("Refusing to overwrite"));
        assert_eq!(std::fs::read_to_string(&settings_path).unwrap(), original);
    }

    #[test]
    fn codex_config_preserves_unrelated_tables() {
        let temp = tempfile::tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let config_path = codex_dir.join("config.toml");
        std::fs::write(
            &config_path,
            r#"[theme]
name = "night"
"#,
        )
        .unwrap();

        register_codex_mcp(&codex_dir, "/opt/agent007", false).unwrap();

        let root: toml::Value =
            toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(root["theme"]["name"].as_str(), Some("night"));
        assert_eq!(
            root["mcp_servers"]["agent007"]["command"].as_str(),
            Some("/opt/agent007")
        );
    }

    #[test]
    fn zed_tasks_merge_without_clobbering_existing_tasks() {
        let temp = tempfile::tempdir().unwrap();
        let zed_dir = temp.path().join(".zed");
        std::fs::create_dir_all(&zed_dir).unwrap();
        let tasks_path = zed_dir.join("tasks.json");
        std::fs::write(
            &tasks_path,
            serde_json::to_string_pretty(&json!([
                {
                    "label": "custom task",
                    "command": "echo",
                    "args": ["hello"]
                },
                {
                    "label": "agent007: run task",
                    "command": "custom-agent007",
                    "args": ["run"]
                }
            ]))
            .unwrap(),
        )
        .unwrap();

        register_zed_tasks(&zed_dir, false).unwrap();

        let tasks: JsonValue =
            serde_json::from_str(&std::fs::read_to_string(&tasks_path).unwrap()).unwrap();
        let entries = tasks.as_array().unwrap();
        assert!(entries.iter().any(|entry| entry["label"] == "custom task"));
        assert!(entries
            .iter()
            .any(|entry| entry["label"] == "agent007: dashboard"));
        let run_task = entries
            .iter()
            .find(|entry| entry["label"] == "agent007: run task")
            .unwrap();
        assert_eq!(run_task["command"], "custom-agent007");
    }

    #[test]
    fn built_in_llm_council_workflow_validates() {
        let raw_protocol_marker_count = WORKFLOW_LLM_COUNCIL
            .matches(WORKFLOW_OPERATING_PROTOCOL_MARKER)
            .count();
        let rendered = render_builtin_workflow_template(WORKFLOW_LLM_COUNCIL);
        assert!(!rendered.contains(WORKFLOW_OPERATING_PROTOCOL_MARKER));

        let def: agent007_workflows::WorkflowDef = serde_yaml::from_str(&rendered).unwrap();
        assert_eq!(def.name, "llm-council");
        assert_eq!(def.steps.len(), 22);
        assert_eq!(raw_protocol_marker_count, def.steps.len());
        assert_eq!(
            rendered.matches("Operating protocol:").count(),
            def.steps.len()
        );

        let registry = agent007_personas::PersonaRegistry::built_in();
        use agent007_core::PersonaProvider;
        for step in &def.steps {
            assert!(
                registry.get(&step.agent).is_some(),
                "missing built-in persona for step {} agent {}",
                step.id,
                step.agent
            );
        }

        agent007_workflows::dag::DagValidator::new(&def)
            .validate()
            .unwrap();
    }

    #[test]
    fn zed_rules_force_preserves_existing_agents_and_writes_generated_companion() {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path();
        let agents_path = project_dir.join("AGENTS.md");
        std::fs::write(&agents_path, "local custom guidance").unwrap();

        register_zed_rules(project_dir, true).unwrap();

        let content = std::fs::read_to_string(&agents_path).unwrap();
        assert_eq!(content, "local custom guidance");

        let rules_path = project_dir.join(".rules");
        let rules = std::fs::read_to_string(&rules_path).unwrap();
        assert!(rules.contains("AGENT007.md"));

        let generated_path = project_dir.join("AGENTS.agent007.generated.md");
        let generated = std::fs::read_to_string(&generated_path).unwrap();
        assert!(generated.contains("Codex reads `AGENTS.md` automatically"));
        assert!(generated.contains("AGENT007.md"));

        let shared_path = project_dir.join("AGENT007.md");
        let shared = std::fs::read_to_string(&shared_path).unwrap();
        assert!(shared.contains("## Routing ladder"));
        assert!(shared.contains("Hosted-MCP reality"));
        assert!(shared.contains("agent007_context_compile"));
        assert!(shared.contains("The durable file/log inspection rule"));
        assert!(shared.contains("agent007_etr_call"));
        assert!(shared.contains("If shell or Python is used for reading/analyzing files"));
        assert!(shared.contains("etr.artifact_read"));
        assert!(shared.contains("etr.workflow_status_summary"));
    }

    #[test]
    fn claude_project_files_write_wrapper_and_all_subagents() {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path();
        let claude_dir = project_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        register_claude_project_files(project_dir, &claude_dir, false).unwrap();

        let claude_md = std::fs::read_to_string(project_dir.join("CLAUDE.md")).unwrap();
        assert!(claude_md.contains("@AGENTS.md"));
        assert!(claude_md.contains("@AGENT007.md"));
        assert!(claude_md.contains("agent007_etr_call"));
        assert!(claude_md.contains("why ETR was not sufficient"));

        let agent_names = [
            "agent007-architect.md",
            "agent007-analyst.md",
            "agent007-planner.md",
            "agent007-builder.md",
            "agent007-reviewer.md",
            "agent007-frontend-designer.md",
            "agent007-release-manager.md",
        ];
        for filename in agent_names {
            assert!(
                claude_dir.join("agents").join(filename).exists(),
                "missing generated Claude sub-agent {filename}"
            );
        }
    }

    #[test]
    fn cursor_and_copilot_project_files_use_host_native_paths() {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path();

        register_cursor_project_files(project_dir, false).unwrap();
        register_copilot_project_files(project_dir, false).unwrap();
        register_workspace_mcp_json("/opt/agent007", project_dir, false).unwrap();

        let cursor_rule =
            std::fs::read_to_string(project_dir.join(".cursor/rules/agent007.mdc")).unwrap();
        assert!(cursor_rule.contains("alwaysApply: true"));
        assert!(cursor_rule.contains("@AGENTS.md"));
        assert!(cursor_rule.contains("@AGENT007.md"));
        assert!(cursor_rule.contains("agent007_etr_call"));

        let copilot =
            std::fs::read_to_string(project_dir.join(".github/copilot-instructions.md")).unwrap();
        assert!(copilot.contains("GitHub Copilot"));
        assert!(copilot.contains("AGENT007.md"));
        assert!(copilot.contains("agent007_etr_call"));
        assert!(copilot.contains("why ETR was not sufficient"));

        let workspace_mcp: JsonValue =
            serde_json::from_str(&std::fs::read_to_string(project_dir.join(".mcp.json")).unwrap())
                .unwrap();
        assert_eq!(
            workspace_mcp["mcpServers"]["agent007"]["command"],
            "/opt/agent007"
        );
    }

    #[test]
    fn codex_project_note_reinforces_etr_first_fallback_rule() {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path();

        register_codex_project_files(project_dir, false).unwrap();

        let codex_note = std::fs::read_to_string(project_dir.join("AGENT007-CODEX.md")).unwrap();
        assert!(codex_note.contains("AGENTS.md"));
        assert!(codex_note.contains("AGENT007.md"));
        assert!(codex_note.contains("agent007_etr_call"));
        assert!(codex_note.contains("If shell/Python is used as a reader/parser"));
        assert!(codex_note.contains("why ETR was not sufficient"));
    }

    #[test]
    fn global_zed_registration_skips_project_rule_files() {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path();
        let zed_dir = temp.path().join("zed-global");

        register_zed(&zed_dir, project_dir, true, false).unwrap();

        assert!(!project_dir.join(".rules").exists());
        assert!(!project_dir.join("AGENTS.md").exists());
    }

    #[tokio::test]
    async fn project_init_builds_initial_repo_index() {
        let temp = tempfile::tempdir().unwrap();

        // Guard that restores the working directory even on panic/assertion failure.
        struct DirGuard(std::path::PathBuf);
        impl Drop for DirGuard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }
        let _guard = DirGuard(std::env::current_dir().unwrap());

        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(
            temp.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        std::env::set_current_dir(temp.path()).unwrap();

        let config = std::sync::Arc::new(crate::config::Config::default());
        execute(config, false, false, false, false, false, false, false)
            .await
            .unwrap();

        let index_path = temp.path().join(".agent007/runtime/repo_index_v2.redb");
        assert!(
            index_path.exists(),
            "expected repo index at {}",
            index_path.display()
        );
        let index = agent007_core::RepoIndex::open(&index_path).unwrap();
        assert_eq!(index.symbol_lookup("alpha", true).unwrap().len(), 1);
        assert!(
            !temp
                .path()
                .join(".agent007/runtime/repo_graph_v1.json")
                .exists(),
            "init should not create legacy repo_graph_v1.json by default"
        );
    }
}

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

## Simple Command Mode (Recommended in Codex)

When users type command-like requests (`$agent007 ...`, `/agent007 ...`, `@agent007 ...`),
prefer `mcp__agent007__agent007_dispatch`.

Examples:

```
$agent007 wf tdd add login rate limiting
$agent007 workflow code-review review current diff
$agent007 skill /brainstorm onboarding ideas
$agent007 /dev-pr-review review this patch
$agent007 run refactor auth module
```

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
| Simple command-style routing | `mcp__agent007__agent007_dispatch` |
| Run any task | `mcp__agent007__agent007_run` |
| List workflows | `mcp__agent007__agent007_workflow_list` |
| Run a workflow | `mcp__agent007__agent007_workflow_run` |
| List skills | `mcp__agent007__agent007_skill_list` |
| Run a skill | `mcp__agent007__agent007_skill_run` |
| List personas | `mcp__agent007__agent007_persona_list` |
| Submit a task | `mcp__agent007__agent007_task_submit` |
"#;
