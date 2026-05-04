# agent007 — AI Orchestration Rules

You have access to the **agent007** MCP server via `context_servers.agent007`.
Always prefer agent007 tools over ad-hoc code generation for complex tasks.
For non-trivial work, route through `agent007_dispatch` before free-form generation.
Runtime mode in editor integrations is typically **hosted-mcp**: the host LLM executes
steps, `agent007` tracks the run, and memory improves over time.

---

## The Core Cycle

```text
1. TASK
   -> user asks for work

2. CONTROL
   -> use agent007_run / agent007_skill_run / agent007_workflow_run
   -> get a run, prompt, or structured plan

3. WORK
   -> execute with the normal editor tools
   -> read files, edit code, run commands, inspect diffs

4. RECORD
   -> when hosted flows ask for it, call agent007_record_tokens
   -> this updates dashboard metrics and preserves output in memory

5. LEARN
   -> future runs can reuse repo brain, memory, and prior outputs
```

The important rule is:

```text
for multi-step or high-context work, route through agent007 first
```

---

## Core Tools

| Tool | Purpose |
|------|---------|
| `agent007_run` | Run a quick task through the full agent stack |
| `agent007_skill_list` | Discover installed skills |
| `agent007_skill_run` | Run a named skill by trigger |
| `agent007_workflow_list` | List available workflows |
| `agent007_workflow_run` | Run a full workflow synchronously |
| `agent007_workflow_start` | Start a hosted workflow session |
| `agent007_workflow_next` | Fetch next ready hosted workflow steps |
| `agent007_workflow_submit_step` | Submit output for a hosted step |
| `agent007_workflow_approve` | Record an approval decision |
| `agent007_record_tokens` | Close the hosted loop and persist output |
| `agent007_context_compile` | Pull repo brain + memory + relevant files |
| `agent007_memory_read` | Read saved memory |
| `agent007_memory_write` | Persist high-signal context |
| `agent007_run_history` | Review prior runs |
| `agent007_repo_brain_refresh` | Rebuild project summary memory |

If the exact tools differ over time, use `agent007_help`, `agent007_skill_list`,
and `agent007_workflow_list` as the source of truth.

---

## Routing Guidance

```text
Quick ad-hoc task
  -> agent007_run

Focused repeatable prompt pattern
  -> agent007_skill_run

Feature delivery / code review / ideation / security / TDD
  -> agent007_workflow_run

Unsure what exists
  -> agent007_skill_list or agent007_workflow_list
```

Recommended workflow routing:

| Workflow | When to use |
|----------|-------------|
| `tdd` | Writing or fixing a feature test-first |
| `code-review` | Reviewing correctness, security, performance, style |
| `sparc` | End-to-end feature execution |
| `feature` | Full delivery with review and approval gates |
| `ideation` | Research to PRD to architecture to plan |
| `brainstorm` | Lightweight ideation before committing to architecture |
| `log-analysis` | Error and incident investigation |
| `security-audit` | Deep security review |

---

## Working Rules

1. For any complex task, prefer `agent007_context_compile` before broad edits.
2. For hosted workflows, keep the user in the loop at approval points.
3. When a hosted task asks for `agent007_record_tokens`, include the final output text so
   memory and dashboard state stay useful.
4. Treat the dashboard as telemetry and run inspection, not the primary planning brain.
5. Preserve user-owned project instructions; update only the agent007-managed guidance.

---

## Project Context

Fill this section in for the current repository:

- Stack:
- Key build/test commands:
- MCP server command:
- Dashboard URL:
- Important modules or directories:
- Delivery constraints:
- Review standards:

Default local commands:

- LSP server: `agent007 serve-lsp --stdio`
- MCP server: `agent007 serve --no-dashboard`
- Full MCP + dashboard: `agent007 serve`
- Web dashboard: `http://localhost:8007`
