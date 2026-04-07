# ADR-007: Auto-Insight Generation from Skill Feedback Patterns

**Date:** 2026-04-07  
**Status:** Accepted  
**Deciders:** agent007 core team

## Context

agent007 records a `FeedbackEntry` for every skill execution via `FeedbackCollector`. Each entry captures the skill name, model, outcome (`Success`, `Failure{reason}`, `ToolError{tool}`), reward score, and timestamp.

Over repeated sessions, skills used frequently on the same project accumulate hundreds of entries. This data contains implicit procedural knowledge — for example, "when skill X fails, it is almost always because of Y". This knowledge currently lives only in raw event records and is never surfaced to the user or fed back into skill prompts.

Two approaches were considered for distilling this knowledge automatically:

1. **Source-level InsightGenerator** — a Rust component in the `learning` crate that watches `FeedbackCollector` output and writes procedural memory entries when patterns are detected.
2. **Workflow-level approximation** — a project-local YAML workflow that re-reads memory file text and asks an LLM to find patterns.

## Decision

Implement **`InsightGenerator`** as a first-class component in `crates/learning/src/insight.rs`.

`InsightGenerator` is attached to `FeedbackCollector` via `with_insight_generator(Arc<InsightGenerator>)`. After each `TaskCompleted` feedback entry is recorded, the collector checks whether the skill's total feedback count is a multiple of `check_every_n`. If so, `InsightGenerator::maybe_generate(skill_name, store)` is called inline (still within the async event loop — see rationale below).

`maybe_generate` triggers only when:
- `feedback_count >= min_feedback_count` (default: 5)
- `failure_rate >= min_failure_rate` (default: 0.2)
- The per-skill insight cap has not been reached (default: 10)

When triggered, it calls the configured LLM model to produce a concise procedural rule, then writes a `type: procedural` memory entry to the `project` scope. The entry is immediately available to `{{memory.project}}` and `{{rag_context}}` in subsequent skill and workflow prompts.

## Rationale

### Why source-level, not a workflow approximation?

| Property | Source-level InsightGenerator | Workflow approximation |
|---|---|---|
| Sees real event data | ✅ skill name, model, failure reason per execution | ❌ only reads existing memory text |
| Runs automatically | ✅ event-driven, no user action needed | ❌ requires manual trigger or hook |
| Works across all projects | ✅ compiled into binary | ❌ project-local YAML only |
| Pattern detection quality | ✅ structured Outcome enum, accurate counts | ❌ LLM guessing from prose |
| Architecturally sound | ✅ | ❌ approximation |

### Why inline async, not spawned task?

The `FeedbackCollector::run()` event loop is a background tokio task. Calling `maybe_generate` inline (with `.await`) adds latency only when the insight check fires — which is infrequent (every `check_every_n` events, only above the failure threshold). The LLM call (~1–2 s) is acceptable in this context since:

- The collector is not on the hot path for user-facing responses.
- Spawning would require `InsightGenerator: Clone + Send + 'static`, adding lifecycle complexity.
- Inline execution preserves sequential ordering with the feedback write (insight is generated from the same store state that was just updated).

If per-project latency requirements change, the call can be trivially moved to `tokio::spawn` by wrapping `InsightGenerator` in `Arc` and cloning before the spawn.

### Why the project memory scope?

Insights are project-specific behavioral patterns. Writing to the `project` scope means:
- They appear in `{{memory.project}}` (loaded into every skill/workflow prompt).
- They are indexed by the RAG retriever for `{{rag_context}}`.
- They are visible in the web dashboard's memory browser.
- They can be manually reviewed, edited, or deleted without touching learning internals.

Writing to the `learning` scope (alongside raw `FeedbackEntry` records) would require special-casing in every prompt template and would not be served by `{{memory.project}}`.

## Alternatives Considered

| Alternative | Reason Not Chosen |
|---|---|
| **Nightly batch job** (`InsightGenerator::run_all()` on a schedule) | Adds a background timer and requires persistent state for "last run" tracking. The per-event check is simpler and surfaces insights incrementally without a scheduler. |
| **Hook-based trigger** (`post_task_complete` hook calls a CLI command) | Hooks run shell commands with no access to the in-process `LearningStore`. They cannot read `FeedbackEntry` structs. This would require serializing the entire store to disk in a separate format just for the hook — over-engineered. |
| **Workflow approximation** (YAML workflow, re-reads memory text) | Does not have access to structured event data. Pattern detection quality is lower. Requires manual invocation. Rejected as the wrong layer. |
| **LLM-free heuristic** (pure Rust, no model call) | Would generate templated strings ("skill X fails 40% of the time due to: …") without the nuance of actionable advice. Useful as a fallback but insufficient as the primary mechanism. |

## Consequences

### Positive

- Procedural insights surface automatically after recurring failures — users get actionable advice without reviewing raw feedback logs.
- Insights are written as standard `type: procedural` memory entries — no new schema, no new prompt variables needed.
- `InsightGenerator` is fully opt-in: passing `None` (no `with_insight_generator` call) leaves `FeedbackCollector` behaviour unchanged.
- Composable with `PromptOptimizer`: both components read `LearningStore` independently. Insights inform users; `PromptOptimizer` refines the skill prompt template. They address different failure modes.
- Full test coverage in `insight.rs`: 6 unit tests cover threshold, failure rate, cap, model failure, and index persistence.

### Negative / Tradeoffs

- **LLM cost per insight**: each generated insight costs one model call (~500 input + ~150 output tokens). With defaults (`check_every_n=5`, `min_failure_rate=0.2`), a skill that fails 30% of the time generates an insight every 5 executions — approximately once per active session. Cost is low in practice.
- **Inline async latency**: the model call adds ~1–2 s to the event loop on trigger. Acceptable for a background collector; unacceptable if the collector is ever moved to the synchronous hot path.
- **No deduplication across similar insights**: if the same failure pattern persists, similar insights are written on each trigger (up to `max_insights_per_skill`). A future improvement could embed and compare new insight text against existing ones before writing.

## Related ADRs

- ADR-005 — Skills as Markdown with frontmatter (insights are written in the same format)
- ADR-006 — Synchronous hook execution (hooks cannot replace InsightGenerator — different layer)
