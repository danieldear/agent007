# PRD: Agent007 Workflow Speed & Efficiency

**Status:** Draft  
**Version:** 0.1  
**Author:** agent007 Architect  

---

## 1. Problem Statement

Agent007 workflows are slow and token-expensive because every step that touches data or runs computation requires a full LLM round-trip (3–15 s API latency). A 40-step workflow costs 20–30 min of pure wait time. The `ftm-analysis` workflow (12 steps, 7-step serial critical path) burns ~120 K tokens — 67% of which are deterministic work that an LLM has no business doing.

Three distinct problems compound each other:

| # | Problem | Impact |
|---|---|---|
| P1 | Every bash/tool call = 1 LLM turn | Latency multiplier: O(n) API calls |
| P2 | No deterministic execution layer | Token bloat on grep/CSV/JSON/math |
| P3 | Hosted-MCP step delivery was broken | Entire workflow execution path was unusable |
| P4 | `task_submit` is a no-op in hosted-MCP | Parallel dispatch impossible, forces inline execution |
| P5 | No step caching | Expensive deterministic steps re-run on every invocation |

---

## 2. Goals

1. **Reduce p95 workflow latency by ≥ 50%** for workflows containing ≥ 5 deterministic steps.
2. **Reduce token consumption by ≥ 60%** on deterministic work (grep, CSV slice, JSON extract, math, file I/O).
3. **Make hosted-MCP step delivery reliable** — no silent stuck states, deterministic re-delivery.
4. **Enable true parallel step dispatch** in hosted-MCP mode via `task_submit`.
5. **Skip unchanged deterministic steps** via content-addressed caching.

### Non-goals (v1)
- Streaming output from ETR tools
- LLM-authored plugin persistence (session-scoped plugins only in v1)
- Cross-user plugin registry / marketplace
- Changing the workflow YAML format in a breaking way

---

## 3. User Stories

### P1 — Serialized LLM Round-trips

> **As a** workflow author,  
> **I want** deterministic steps (file reads, regex, CSV slicing, JSON extraction, arithmetic) to run without consuming an LLM turn,  
> **so that** a 40-step workflow does not take 30 minutes.

**Acceptance criteria:**
- A step marked `type: extract` executes via the Embedded Tool Runtime (ETR) with zero LLM API calls.
- Latency for an L1 ETR call is < 200 ms (no LLM, no subprocess startup).
- An L2 plugin call completes in < 2 s (subprocess startup amortized).
- A workflow with 10 ETR steps and 5 LLM steps finishes in ≤ 5 × (avg LLM latency), not 15 ×.

---

### P2 — Embedded Tool Runtime (ETR)

> **As a** workflow author,  
> **I want** a typed, policy-governed tool registry (L1 built-ins + L2 plugins + L3 gated shell),  
> **so that** I can replace freeform bash blocks with compact structured calls and reduce the tokens the host LLM sees.

**Acceptance criteria:**
- `agent007_etr_call` MCP tool is available in all execution modes.
- **L1 tools are native Rust** (compiled into the agent007 binary, no subprocess, < 1 ms latency, zero external runtime dependency): `etr.grep`, `etr.json_extract`, `etr.csv_slice`, `etr.glob`, `etr.file_stat`, `etr.math`, `etr.diff`.
- **L2 plugins are language-agnostic** — the Rust framework owns the contract (manifest schema, I/O protocol, policy, compaction); plugin bodies can be written in Python, Shell, Node.js, Ruby, compiled Go, WebAssembly, or any language that reads JSON from stdin and writes JSON to stdout.
- All ETR outputs pass through the compactor — large outputs are summarised, not dumped verbatim.
- Every call writes to `.agent007/runtime/etr_audit.jsonl` (tool name, inputs, policy decision, output size, latency).
- The policy engine rejects path traversal outside declared `allowed_paths` — enforced at the Rust boundary, not inside the plugin.
- `etr.list` returns the full registry so the LLM can discover available tools.
- L2 plugin manifests (TOML + any-language entry point) can be installed via `agent007 etr install`.
- L3 gated shell is off by default; enabled via `allow_l3_shell = true` in config; always-compacted.

**Token savings target:**
- FTM analysis session: 120 K → 40 K tokens (67% reduction on deterministic work).
- Measured by comparing token counts for `ftm-analysis` workflow runs with/without ETR.

**Output compaction rules:**

| Output type | Strategy |
|---|---|
| Short JSON (< 2 KB) | Return as-is |
| Large JSON | Return `compact_key` subpath only + `"truncated": true` |
| Tabular (CSV/rows) | First N rows + column summary + row count |
| Log text | First M + last M lines + total line count |
| Binary | File stat only (size, hash) |
| Error / traceback | Last 20 lines + exception type |

---

### P3 — Hosted-MCP Step Delivery Reliability

> **As a** Copilot / Claude / LLM host driving a workflow via MCP,  
> **I want** `workflow_next` to always return ready step prompts — even if the server has already marked them `Running`,  
> **so that** I never end up in a stuck state where steps run forever but are never delivered.

**Status: FIXED** in commit `4c72045e1`.

**Acceptance criteria (regression prevention):**
- `workflow_next` is idempotent — calling it multiple times on a Running step re-delivers the same prompt.
- Steps never transition `Running → stuck` without an explicit timeout or failure event.
- A regression test (`workflow_next_redelivers_running_steps_when_host_polls_again`) passes in CI.
- `workflow_status` accurately reflects step state without side effects.

**Root cause (documented):**
- `workflow_start` → `dispatch()` marked steps `Running` AND returned them.
- Subsequent `workflow_next` hit the `!running_steps.is_empty()` guard → returned `AwaitingOutputs` with empty `ready_steps`.
- Fix: moved `step_map` construction before the guard; `dispatch_ready=true` path now re-delivers Running step prompts.

---

### P4 — task_submit Background Dispatch in Hosted-MCP Mode

> **As a** workflow step running inside a hosted-MCP session,  
> **I want** `task_submit` to dispatch work to a real background subprocess with the requested persona applied,  
> **so that** I can parallelise sub-tasks without consuming my own context window.

**Current behavior:** `task_submit` returns "execute directly" in hosted-MCP mode — no subprocess, no async handle, persona has no effect.

**Acceptance criteria (v1 — documentation + workaround):**
- `task_submit` response in hosted-MCP mode clearly documents the limitation.
- Response recommends the correct workaround: use the host environment's `task` tool with `agent_type: general-purpose`.
- Response explains that `workflow_submit_step` should be called by the spawned agent to report results back.

**Status: PARTIALLY FIXED** in commit `4c72045e1` (documentation improved).

**Acceptance criteria (v2 — real dispatch):**
- `agent007_task_submit` in hosted-MCP mode spawns an actual subprocess with the persona's system prompt applied.
- Returns a `task_id` that can be polled via `agent007_task_status(task_id)`.
- Subprocess calls `agent007_workflow_submit_step` on completion if a `workflow_session` + `step_id` are passed.
- Persona's `preferred_model` and `allowed_tools` are applied to the spawned worker.

---

### P5 — Step Caching

> **As a** workflow author running iterative analysis,  
> **I want** deterministic steps whose inputs haven't changed to return cached results instantly,  
> **so that** re-running a workflow after a small edit doesn't re-execute expensive unchanged steps.

**Acceptance criteria:**
- Each step has an optional `cache: true` flag in the workflow YAML.
- Cache key = SHA256 of (step id + rendered prompt + all referenced file contents).
- Cache hit returns the previous output instantly with a `"cached": true` flag in the response.
- Cache miss executes normally and stores the result.
- Cache entries are stored under `.agent007/runtime/step_cache/` as content-addressed JSON blobs.
- Cache is invalidated automatically when any input file changes.
- `agent007 cache clear` purges all cached step outputs.
- Cache hits are surfaced in the dashboard's step timing view.

---

## 4. Architecture Summary

```
Workflow YAML
  └── step (type: extract, cache: true)
        │
        ▼
  ETR Dispatcher (Rust MCP handler)
        │
   ┌────┴──────────────────────────┐
   │                               │                               │
   ▼                               ▼                               ▼
L1: Built-in tools           L2: Plugins                    L3: Gated shell
(etr.grep, etr.csv_slice,    (.agent007/plugins/            (allow_l3_shell = true,
 etr.json_extract, …)         manifest.toml + any language)  deny-pattern check,
 Pure Rust, in-process.       stdin/stdout JSON contract.    rate-limited, compacted)
 < 1 ms.                      Subprocess. Any executor.
                               < 2 s.
        │
        ▼
  Policy Engine
  (path binding, network, subprocess, rate limit, deny patterns)
        │
        ▼
  Output Compactor
  (compact_key, tabular truncation, log head+tail)
        │
        ▼
  Audit Log (.agent007/runtime/etr_audit.jsonl)
        │
        ▼
  Step Cache (content-addressed, .agent007/runtime/step_cache/)
        │
        ▼
  LLM response (compact JSON, never raw shell output)
```

---

## 5. Implementation Phases

### Phase 1 — ETR Foundation + `type: extract` (v1 scope)
- Implement `agent007_etr_call` MCP tool in `crates/cli/src/commands/serve.rs`.
- Ship L1 tools as **native Rust** (no subprocess, no external runtime): `etr.grep` (`grep-searcher`), `etr.json_extract` (`serde_json`+`jaq`), `etr.csv_slice` (`csv`), `etr.glob` (`globset`), `etr.file_stat` (`std::fs`), `etr.math` (`evalexpr`), `etr.diff` (`similar`).
- Implement output compactor (JSON, tabular, log strategies).
- Write audit log.
- Add `type: extract` to workflow YAML schema.
- Wire `type: extract` steps to ETR dispatcher in `crates/workflows/src/hosted.rs`.
- No L2/L3 yet.

**Deliverable:** LLM can replace `bash grep …` + parse with `etr.grep` + compact JSON. Zero Python dependency.

### Phase 2 — Step Caching
- Implement content-addressed cache in `crates/workflows/src/cache.rs`.
- Add `cache: true` flag to workflow YAML step spec.
- Expose cache hits in dashboard step timing.
- Add `agent007 cache clear` CLI command.

**Deliverable:** Re-running `ftm-analysis` after editing the synthesis prompt skips all unchanged ETR steps.

### Phase 3 — L2 Plugin Registry (Language-Agnostic)
- Define manifest TOML schema + Rust validator.
- Implement plugin loader + subprocess launcher (stdin/stdout JSON protocol).
- Implement path-binding jail enforced at Rust boundary (not inside plugin).
- Implement `agent007 etr install` / `uninstall`.
- Migrate `ftm_burst_report.py` to L2 plugin (`executor = "python"`) as reference example.

**Deliverable:** Any developer can write a plugin in Python, Shell, Node, or any language. FTM workflow calls `etr.ftm_burst_summary` instead of embedding Python paths.

### Phase 4 — task_submit Real Dispatch (hosted-MCP v2)
- Implement subprocess worker pool in agent007 MCP server.
- `task_submit` spawns real worker; returns `task_id`.
- `task_status` polls worker state.
- Worker calls `workflow_submit_step` on completion.

**Deliverable:** Workflow steps can fan out to parallel workers without consuming host LLM context.

### Phase 5 — L3 Gated Shell + Self-Extension
- Implement deny-pattern checker + rate limiter.
- Gate behind config flag.
- `etr.list` returns full schema — enables LLM to write plugins.
- Session-scoped plugin registration.

---

## 6. Success Metrics

| Metric | Baseline | Target | How measured |
|---|---|---|---|
| `ftm-analysis` p95 latency | ~25 min | < 12 min | Workflow run timer |
| Token consumption (deterministic work) | ~120 K | < 45 K | Dashboard token counter |
| Stuck workflow rate (hosted-MCP) | ~100% (was broken) | 0% | Regression test suite |
| Cache hit rate (re-run same workflow) | 0% | > 70% | Cache stats in dashboard |
| L1 ETR call latency | N/A | < 200 ms | Audit log timestamps |

---

## 7. Risks & Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Path traversal in L2 plugins | Medium | Policy engine validates `allowed_paths` before any filesystem call |
| ETR adds latency for small outputs | Low | L1 in-process: no subprocess boundary; < 200 ms target |
| Cache stale on file rename | Medium | Cache keys include file mtime; rename invalidates by hash mismatch |
| Subprocess pool leaks in task_submit (Phase 4) | Medium | Worker timeout (configurable, default 5 min); reap on MCP disconnect |
| Breaking workflow YAML compatibility | Low | `type: extract` and `cache: true` are additive optional fields |

---

## 8. Open Questions

1. **Venv isolation** — shared ETR venv (faster) vs per-plugin venv (safer)? Recommend shared with conflict detection at install time.
2. **MCP server reload after plugin install** — restart required or lazy reload? Recommend lazy: rescan `plugin_registry.json` on `etr.list`/`etr.install`.
3. **Cross-session plugin sharing** — project-scoped (`.agent007/plugins/`) only, or user-global (`~/.agent007/plugins/`) too? Recommend both, project takes precedence.
4. **Dashboard cache visibility** — should cached step outputs be viewable in the step detail panel? Recommend yes (click to expand cached output).
5. **task_submit Phase 4 model** — worker pool in MCP server process, or fork per task? Recommend fork-per-task with a max concurrency cap (default 4).

---

## 9. Related Documents

- `tool-runtime-design.md` — ETR architecture design doc (detailed L1/L2/L3 spec, SwiftBash comparison)
- `docs/architecture.md` — overall agent007 architecture
- `crates/workflows/src/hosted.rs` — hosted-MCP workflow engine (P3 fix lives here)
- Commit `4c72045e1` — P3 + P4 documentation fixes
- Commit `8dfb5d50d` — dashboard Invalid Date fix
