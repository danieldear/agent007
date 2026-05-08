# agent007 Embedded Tool Runtime (ETR) — Design Document

## 1. Problem Statement

The host LLM in an agent007 session burns tokens on three categories of work:

| Category | Token cost | Can be offloaded? |
|---|---|---|
| Reasoning, synthesis, narrative | High, irreducible | No |
| Deterministic computation (grep, sort, arithmetic) | Medium | Yes — completely |
| Boilerplate orchestration (file I/O, JSON extraction, shell one-liners) | Low–medium | Yes — mostly |

Currently, only git operations are offloaded to a built-in tool. Everything else is either a prompt-driven bash call (untyped, no policy, no compaction) or a manually maintained Python script. This means:
- The LLM must *describe* the computation, emit a bash block, parse the output, and retry on errors — all at full token cost.
- There is no structured security boundary between agent-owned tools and host-OS shell access.
- Adding new capabilities requires editing YAML workflow files or writing freeform bash, with no discoverable registry.

The ETR is the layer that changes this.

---

## 2. Goals

1. **Token reduction** — deterministic work (file reads, CSV slicing, JSON extraction, regex, math) executes inside the runtime with compact structured outputs, not as multi-turn LLM bash sessions.
2. **LSP-style discoverability** — tools are registered with typed manifests. The LLM can enumerate, describe, and call tools with the same structured syntax it uses for MCP tools.
3. **Plugin management** — new tools can be written in any language (Python, Shell, Node.js, Go, Ruby, WASM, …) and installed without editing agent007 core. The Rust framework defines the contract; plugins implement it.
4. **Security** — a policy engine gates every call before execution. No tool escapes its declared permission set.
5. **Auditability** — every call writes to an append-only audit log with inputs, outputs, and policy decision.

---

## 3. Architecture Overview

```
┌────────────────────────────────────────────────────────────────┐
│                        Host LLM (agent007)                     │
│                                                                │
│   synthesize / reason / plan                                   │
│       │                                                        │
│       ▼                                                        │
│   ETR Tool Call (structured JSON, typed)                       │
│       │                                                        │
└───────┼────────────────────────────────────────────────────────┘
        │
        ▼
┌────────────────────────────────────────────────────────────────┐
│                    ETR Dispatcher                              │
│                                                                │
│   1. Parse manifest — validate types, required fields          │
│   2. Policy engine — check permissions, path bindings          │
│   3. Route to execution layer (L1 / L2 / L3)                   │
│   4. Compact output — truncate, summarize, redact              │
│   5. Audit log write                                           │
└───────┬────────────────────────────────────────────────────────┘
        │
   ┌────┴──────────────────────┐
   │                           │                           │
   ▼                           ▼                           ▼
L1: Deterministic Built-ins  L2: Registered Plugins    L3: Gated Shell
  (Rust, in-process,           (TOML manifests +           (explicit allow-list,
   zero dependencies,           any-language bodies,        audit required,
   grep, json_extract,          stdin/stdout JSON,          full output compaction)
   csv_slice, diff, math, …)    installed under
                                .agent007/plugins/
```

---

## 4. Execution Layers

### L1 — Deterministic Built-ins

Native Rust functions compiled directly into the agent007 binary. Zero external dependencies, no subprocess boundary, no Python runtime required. Always available, always trusted. Call latency < 1 ms.

| Tool | Rust crate | Input | Output |
|---|---|---|---|
| `etr.grep` | `grep-searcher` (ripgrep) | `{pattern, path, context_lines}` | `{matches: [{file, line, text}], count}` |
| `etr.json_extract` | `serde_json` + `jaq` | `{path, jq_path}` | `{value}` |
| `etr.csv_slice` | `csv` | `{path, columns, filter_expr, limit}` | `{rows: [...], truncated}` |
| `etr.diff` | `similar` | `{path_a, path_b}` | `{unified_diff, changed_lines}` |
| `etr.math` | `evalexpr` | `{expression}` | `{result}` |
| `etr.file_stat` | `std::fs` | `{path}` | `{size, mtime, exists}` |
| `etr.glob` | `globset` | `{pattern, root}` | `{paths: [...]}` |

All L1 tools return compact JSON. The LLM never sees raw shell output.

### L2 — Registered Plugins (Language-Agnostic)

TOML manifests under `.agent007/plugins/<name>/manifest.toml`. The Rust framework owns the contract (manifest schema, I/O protocol, policy enforcement, output compaction). The plugin body can be written in **any language** — Python, Shell, Node.js, Ruby, Go binary, WebAssembly, or anything that can read JSON from stdin and write JSON to stdout.

**Plugin I/O contract (language-independent):**
1. Rust dispatcher spawns the plugin as a subprocess.
2. Dispatcher writes the validated input as a JSON object to the plugin's `stdin`.
3. Plugin performs its work and writes a JSON object to `stdout`.
4. Plugin exits 0 on success, non-zero on error (stderr is captured for error messages).
5. Rust reads `stdout`, runs it through the output compactor, and returns the result to the LLM.

The plugin never needs to know about agent007 internals. It just reads stdin, processes, writes stdout.

Each manifest declares:

```toml
[tool]
name        = "ftm_burst_summary"
version     = "1.0.0"
description = "Summarize FTM burst-level KPIs from a dataset directory"
executor    = "python"           # python | shell | node | ruby | wasm | binary
entry       = "run.py"           # relative to plugin directory; for "binary" this is the compiled executable

[input]
dataset_root = { type = "path", required = true, binding = "read" }
output_dir   = { type = "path", required = false, binding = "write", default = "{dataset_root}" }
limit_bursts = { type = "integer", required = false, default = 0 }

[output]
format      = "json"
compact_key = "summary"          # the LLM receives output[compact_key] only

[permissions]
allowed_paths  = ["{dataset_root}", "{output_dir}"]
network        = false
subprocess     = false
env_passthrough = []

[examples]
[[examples.call]]
description = "Summarize may5/6690"
input = { dataset_root = "may5/6690" }
```

The dispatcher loads the manifest, validates inputs against the schema, injects `allowed_paths` as a sandbox jail, and invokes the plugin entry point as a subprocess. The plugin receives validated input via stdin JSON and returns its result via stdout JSON. Rust handles all policy enforcement and output compaction — the plugin only needs to implement the computation.

### L3 — Gated Shell

When no L1 or L2 tool covers a need, the LLM can request a gated shell call. This requires:
- An explicit `allow_shell` flag in the active workflow's security policy.
- The command must not match any deny-list pattern (no curl to external hosts, no rm -rf, no eval-style constructs).
- Output is always passed through the ETR compactor before being returned to the LLM.
- The call is logged with the full command and trimmed output.

L3 is the escape hatch. New L3 patterns that recur → candidate for L2 plugin.

---

## 5. LSP-Style Tool Syntax

Tools are called with a structured object matching the MCP tool-call convention the host LLM already uses:

```json
{
  "tool": "etr.csv_slice",
  "input": {
    "path": "may5/6690/rtt_20260501_143200.csv",
    "columns": ["burst_id", "range_m", "t4_del"],
    "filter_expr": "t4_del > 5000",
    "limit": 50
  }
}
```

The dispatcher exposes a single MCP tool `agent007_etr_call` that accepts this object and returns:

```json
{
  "tool": "etr.csv_slice",
  "status": "ok",
  "output": { "rows": [...], "truncated": false },
  "audit_id": "etr-20260510-0043-abc"
}
```

Errors return `"status": "error"` with a `"reason"` field and suggested correction.

### Listing available tools

```json
{ "tool": "etr.list", "input": { "layer": "all" } }
```

Returns a registry snapshot: name, description, input schema summary, permissions summary. The LLM can use this to discover what is available before deciding whether to call an L1, L2, or L3 tool.

---

## 6. Policy Engine

Every call passes through the policy engine before execution. The engine checks:

1. **Path binding** — each declared `path` input is resolved to an absolute path and checked against the tool's `allowed_paths` list. Traversal outside the bound paths is rejected.
2. **Network** — if the manifest declares `network = false`, any network-accessing call in the plugin body is intercepted.
3. **Subprocess** — if `subprocess = false`, the plugin cannot spawn child processes.
4. **Rate limiting** — L3 shell calls are throttled per session (configurable, default 10/min).
5. **Deny patterns** — a global deny list of shell patterns (e.g., `rm -rf /`, `curl .* | bash`, `eval`) is checked for L3 calls.

Policy decisions are written to the audit log immediately, before execution.

### Policy config (`.agent007/config.toml` extension)

```toml
[etr.policy]
allow_l3_shell      = false      # enable gated shell globally
l3_rate_limit       = 10         # calls per minute
audit_log           = ".agent007/runtime/etr_audit.jsonl"
deny_patterns       = [
  "rm\\s+-rf",
  "curl.*\\|.*bash",
  "eval\\s+",
  "\\$\\{[^}]+@",               # parameter transformation
]

[etr.paths]
workspace_root      = "."        # all relative paths resolve inside here
```

---

## 7. Plugin Management

### Installing a plugin

```
agent007 etr install ./my_plugin/          # from local directory
agent007 etr install @agent007/ftm-tools   # from registry (future)
```

The installer:
1. Validates the manifest schema.
2. Checks for dependency collisions in the runtime venv.
3. Installs Python dependencies into `.agent007/runtime/etr_venv/`.
4. Registers the manifest in `.agent007/runtime/plugin_registry.json`.

### Uninstalling

```
agent007 etr uninstall ftm_burst_summary
```

### Writing a new plugin

The LLM itself can write a plugin given a description. The workflow is:
1. LLM calls `etr.list` to see existing tools and naming conventions.
2. LLM writes `manifest.toml` + `run.py` into `.agent007/plugins/<name>/`.
3. LLM calls `agent007 etr install .agent007/plugins/<name>/` to validate and register.
4. New tool appears in `etr.list` output immediately.

This is the self-extension loop: the LLM adds tools to reduce its own future token cost.

---

## 8. Output Compaction

All ETR outputs pass through the compactor before being returned to the LLM. Compaction rules:

| Output type | Strategy |
|---|---|
| Short JSON (< 2 KB) | Return as-is |
| Large JSON | Return `compact_key` subpath only; append `"truncated": true` |
| Tabular (CSV/rows) | Return first N rows + column summary + row count |
| Log text | Return first M lines + last M lines + total line count |
| Binary | Return file stat only (size, hash) |
| Error / traceback | Return last 20 lines of traceback + exception type |

The LLM can request `"compact": false` for a full dump when needed (L1/L2 only; L3 always compact).

---

## 9. Token Savings Model

Real token savings only occur when ALL three conditions are met:

1. **Small input** — the LLM's call message is a compact JSON object, not a raw file dump.
2. **Deterministic execution** — the tool body does not require LLM reasoning; it is pure computation.
3. **Compact output** — the result returned to the LLM is structured and small.

If any condition fails (e.g., the output is large prose that the LLM must read verbatim), ETR adds latency without saving tokens. The compactor and manifest schema exist to enforce conditions 2 and 3. Condition 1 is enforced by making ETR calls typed structured objects, not bash strings.

Estimated savings for an FTM analysis session:
- Without ETR: ~40 LLM turns of `bash` + output parsing = ~120K tokens
- With ETR (L1/L2 for CSV slicing, log grep, JSON extraction): ~15 LLM turns of structured calls + compact JSON = ~40K tokens
- Net: ~67% token reduction on deterministic work; synthesis/narrative cost unchanged

---

## 10. Relationship to SwiftBash

SwiftBash (github.com/Cocoanetics/SwiftBash) is an embedded sandboxed Bash interpreter written in Swift, designed as "the local code interpreter for an LLM agent." Its design is very close to ETR's L3 layer:
- Sandboxed execution with explicit allow-lists
- Typed command kit (`BashCommandKit`) for structured tool composition
- Designed to run inside an LLM agent loop without full OS shell access

**Decision: do not adopt SwiftBash directly.** Reasons:
1. Swift dependency adds a non-trivial build/distribution requirement for a Rust-native tool stack.
2. The ETR L1/L2 layers cover the high-value deterministic cases that SwiftBash targets, without a subprocess boundary for L1.
3. SwiftBash's typed command kit maps cleanly to ETR L2 plugin manifests — if we ever want a sandboxed subprocess executor for L2, SwiftBash (or a minimal equivalent) can serve as the runtime engine for that layer without changing the ETR registry/policy interface.

The ETR design is SwiftBash-inspired at the architectural level (typed manifests, policy, compaction) but the framework is implemented in Rust. L2 plugins can be written in any language that implements the stdin/stdout JSON contract.

---

## 11. Implementation Phases

### Phase 1 — L1 Built-ins + Dispatcher (foundation)

- Implement `agent007_etr_call` MCP tool in agent007 Rust core (`crates/cli/src/commands/serve.rs`).
- Ship L1 tools as native Rust: `etr.grep` (`grep-searcher`), `etr.json_extract` (`serde_json`+`jaq`), `etr.csv_slice` (`csv`), `etr.glob` (`globset`), `etr.file_stat` (`std::fs`), `etr.math` (`evalexpr`), `etr.diff` (`similar`).
- Implement compactor with tabular + JSON strategies.
- Write audit log to `.agent007/runtime/etr_audit.jsonl`.
- No plugin loading, no L3 shell yet.

**Deliverable:** LLM can replace `bash grep …` + parse with `etr.grep` call + compact JSON response. Zero Python dependency.

### Phase 2 — Plugin Registry + L2

- Define manifest TOML schema and validator (in Rust).
- Implement plugin loader: scan `.agent007/plugins/`, validate, register.
- Implement subprocess launcher with stdin/stdout JSON protocol (language-agnostic).
- Implement path-binding jail enforced at the Rust boundary (not inside the plugin).
- Migrate `ftm_burst_report.py` and `ftm_pipeline_bridge.py` to L2 plugin manifests (executor = "python").
- Implement `agent007 etr install` / `uninstall` CLI commands.

**Deliverable:** FTM workflow steps call `etr.ftm_burst_summary` instead of embedding Python paths in YAML. Plugin authors can use Python, Shell, Node, Go binary, or any language — the framework doesn't care.

### Phase 3 — L3 Gated Shell

- Implement deny-pattern checker.
- Implement rate limiter.
- Gate behind `allow_l3_shell = true` in config.
- Compactor always-on for L3 output.

**Deliverable:** LLM can request gated shell for uncovered cases without escaping the policy engine.

### Phase 4 — Self-Extension Loop

- `etr.list` returns full registry with schema.
- LLM-authored plugins validated and registered in session.
- Session-scoped plugins (not persisted) for one-off analysis tools.

**Deliverable:** LLM writes a new plugin during a session, uses it immediately, and the pattern is either discarded or promoted to `.agent007/plugins/` for reuse.

---

## 12. Open Questions

1. **Venv isolation** — only relevant for Python plugins. Recommend shared venv for Python plugins with manifest-declared pip extras, conflict detection at install time. Non-Python plugins have no venv concern.
2. **MCP server reload** — after a new plugin is installed, does the MCP server need a restart? Recommend lazy reload: the dispatcher rescans `plugin_registry.json` on each `etr.list` or `etr.install` call without full restart.
3. **Streaming output** — large tabular outputs may benefit from streaming. Initial version returns synchronously; streaming is a Phase 4+ enhancement.
4. **Cross-session plugin sharing** — plugins under `.agent007/plugins/` are project-scoped. Should there be a user-global plugin store (`~/.agent007/plugins/`)? Recommend: yes, with project plugins taking precedence over global.
