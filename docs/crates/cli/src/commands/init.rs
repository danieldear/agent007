use std::path::{Path, PathBuf};
use anyhow::Result;

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

pub async fn execute(_config: std::sync::Arc<Config>, force: bool, global: bool) -> Result<()> {
    let home = super::run::agent007_home();

    // Determine the .claude/ directory to write MCP registration and commands into.
    // Default: current project's .claude/ (project-scoped, doesn't affect other projects).
    // --global: ~/.claude/ (available in every Claude Code project).
    let claude_scope_dir = if global {
        dirs_home().join(".claude")
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| dirs_home())
            .join(".claude")
    };
    let scope_label = if global { "~/.claude/ (global)" } else { ".claude/ (project)" };

    println!();
    println!("{BOLD}{CYAN}agent007{RESET} — initializing your workspace");
    println!("{DIM}home: {}{RESET}", home.display());
    println!("{DIM}scope: {scope_label}{RESET}");

    // ── 1. Directory structure ──────────────────────────────────────────────
    section("1. Creating directory structure");
    ensure_dir(&home, "~/.agent007/")?;
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

    // ── 4. Built-in workflows ───────────────────────────────────────────────
    section("4. Writing built-in workflows");
    let wf_dir = home.join("workflows");
    write_if_missing(&wf_dir.join("log-analysis.yaml"),  WORKFLOW_LOG_ANALYSIS,  "workflows/log-analysis.yaml")?;
    write_if_missing(&wf_dir.join("code-review.yaml"),   WORKFLOW_CODE_REVIEW,   "workflows/code-review.yaml")?;
    write_if_missing(&wf_dir.join("sparc.yaml"),         WORKFLOW_SPARC,         "workflows/sparc.yaml")?;
    write_if_missing(&wf_dir.join("tdd.yaml"),           WORKFLOW_TDD,           "workflows/tdd.yaml")?;

    // ── 5. Example custom agent ─────────────────────────────────────────────
    section("5. Writing example custom agent");
    let agent_path = home.join("personas").join("my-agent.toml");
    write_if_missing(&agent_path, EXAMPLE_AGENT, "personas/my-agent.toml")?;

    // ── 6. Claude Code MCP registration ────────────────────────────────────
    // Write directly to <scope>/settings.json — no `claude` CLI required.
    // Claude Code reads mcpServers on startup and auto-launches the server.
    section("6. Registering MCP server with Claude Code");
    let binary = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("agent007"));
    register_mcp_in_settings(&binary, &claude_scope_dir, force)?;

    // ── 7. Claude Code command files ───────────────────────────────────────
    section("7. Installing slash commands for Claude Code");
    let commands_dir = claude_scope_dir.join("commands");
    if !commands_dir.exists() {
        std::fs::create_dir_all(&commands_dir)?;
        ok(&format!("{}/commands/ created", scope_label));
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
                // Parse description from frontmatter
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                let description = content
                    .lines()
                    .find(|l| l.starts_with("description:"))
                    .map(|l| l.trim_start_matches("description:").trim().to_string())
                    .unwrap_or_else(|| format!("Run /agent007/{stem} skill"));
                let trigger = format!("/agent007/{stem}");
                let cmd_content = format!(
                    "{description}\n\nUse the mcp__agent007__agent007_skill_run tool with trigger \"{trigger}\" and args \"$ARGUMENTS\".\n"
                );
                std::fs::write(&cmd_file, cmd_content)?;
                installed += 1;
            }
        }
    }
    if installed > 0 {
        ok(&format!("{installed} slash commands installed → {}/commands/", scope_label));
    } else {
        ok("All slash commands already installed");
    }

    // ── 8. Install Claude Code sub-agents ──────────────────────────────────
    // These go in <scope>/.claude/agents/ so Claude Code can spawn them autonomously.
    // The architect agent is the key one: it receives a natural-language instruction,
    // picks the right workflow or composes one dynamically, runs it via MCP, and
    // returns a synthesized report. It persists across sessions via the MCP server.
    section("8. Installing Claude Code sub-agents");
    let agents_dir = claude_scope_dir.join("agents");
    if !agents_dir.exists() {
        std::fs::create_dir_all(&agents_dir)?;
        ok(&format!("{}/agents/ created", scope_label));
    }
    write_if_missing(&agents_dir.join("agent007-architect.md"), CLAUDE_AGENT_ARCHITECT, "agents/agent007-architect.md")?;
    write_if_missing(&agents_dir.join("agent007-analyst.md"),   CLAUDE_AGENT_ANALYST,   "agents/agent007-analyst.md")?;

    // ── 9. Environment check ───────────────────────────────────────────────
    section("9. Environment check");

    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        warn("ANTHROPIC_API_KEY not set — skills will return placeholder responses");
        info("Add to ~/.zshrc:  export ANTHROPIC_API_KEY=sk-ant-...");
        info("Then restart Claude Code to pick it up");
    } else {
        ok(&format!("ANTHROPIC_API_KEY set ({} chars)", api_key.len()));
    }

    let git_ok = std::process::Command::new("git").arg("--version").output().is_ok();
    if git_ok { ok("git available") } else { warn("git not found in PATH") }

    // ── 9. Summary ─────────────────────────────────────────────────────────
    let skill_count = std::fs::read_dir(&home.join("skills"))
        .map(|d| d.flatten().filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md")).count())
        .unwrap_or(0);

    println!();
    println!("{BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}");
    println!("{BOLD}{GREEN}agent007 is ready!{RESET}");
    println!("{BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}");
    println!();
    println!("  Home:        {DIM}{}{RESET}", home.display());
    println!("  Skills:      {GREEN}{skill_count}{RESET} loaded");
    println!("  MCP server:  {GREEN}agent007 serve{RESET}");
    println!("  Dashboard:   {CYAN}agent007 dashboard{RESET}");
    println!();
    println!("{DIM}Quick start:{RESET}");
    println!("  agent007 run \"explain the main function in this project\"");
    println!("  agent007 dashboard");
    println!("  /agent007-explain <code>  (in Claude Code)");
    println!();

    if api_key.is_empty() {
        println!("{YELLOW}Next step: set ANTHROPIC_API_KEY to enable real model execution{RESET}");
        println!();
    }

    Ok(())
}

/// Write (or merge) the agent007 MCP server entry into <claude_dir>/settings.json.
/// `claude_dir` is either the project's `.claude/` or the global `~/.claude/`.
fn register_mcp_in_settings(binary: &Path, claude_dir: &Path, force: bool) -> Result<()> {
    let settings_path = claude_dir.join("settings.json");

    // Canonicalize the binary path so it has no `../` segments — Claude Code
    // needs a clean absolute path regardless of where `agent007 init` was run.
    let canonical = binary
        .canonicalize()
        .unwrap_or_else(|_| binary.to_path_buf());

    // Read existing JSON or start with empty object
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

    // Always update the path if force, or if the path changed (e.g. binary moved)
    let existing_cmd = servers
        .get("agent007")
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let new_cmd = canonical.to_string_lossy();

    if servers.contains_key("agent007") && !force && existing_cmd == new_cmd.as_ref() {
        ok(&format!("agent007 already registered → {}", new_cmd));
        return Ok(());
    }

    let entry = serde_json::json!({
        "command": new_cmd,
        "args": ["serve"]
    });
    servers.insert("agent007".to_string(), entry);

    // Ensure parent dir exists
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&settings_path, serde_json::to_string_pretty(&root)?)?;
    ok(&format!("Wrote mcpServers.agent007 → {}", new_cmd));
    ok(&format!("  config: {}", settings_path.display()));
    println!();
    warn("Restart Claude Code to activate the MCP server in existing sessions");
    info("New Claude Code windows will pick it up automatically");
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
default = "claude-sonnet-4-6"
# To use a different model, change the above.
# Available: claude-opus-4-6, claude-sonnet-4-6, claude-haiku-4-5-20251001

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

const EXAMPLE_AGENT: &str = r#"# Example custom agent persona
# Save to ~/.agent007/personas/<name>.toml

name            = "MyAgent"
description     = "A custom agent for my specific workflow"
preferred_model = "claude-sonnet-4-6"
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
