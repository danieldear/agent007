# PRD: Retrieval-First Reliability Improvements for agent007

## Executive Summary
This PRD defines a near-term product slice to make agent007 outperform direct one-shot LLM calls by improving memory recall, execution consistency, and tool-safety controls while preserving usability in hosted and standalone modes.

## Users
1. Developer using Codex/Claude/Cursor with agent007 MCP.
2. Team lead running repeatable workflows across repositories.
3. Ops maintainer diagnosing run quality regressions.

## User Stories
1. As a developer, I want agent007 to remember project context automatically so I do not restate setup on every task.
2. As a developer, I want `skill run` behavior to match workflow behavior so outputs are consistent.
3. As a maintainer, I want run-level retrieval metrics so I can detect RAG regressions quickly.
4. As a team admin, I want persona tool policies enforced to reduce unsafe/unapproved tool usage.

## Functional Requirements
1. RAG warmup indexing at stack startup for bounded project context.
2. Bounded indexing controls (file count/size/chars and memory caps).
3. Warmup kill switch via `AGENT007_RAG_WARMUP=0`.
4. Unified CLI skill execution path via shared stack executor.
5. Per-run retrieval telemetry artifact.
6. Persona tool-policy runtime evaluation for MCP tool calls.
7. Dashboard run details must show retrieval telemetry and persona policy warnings.

## Non-Functional Requirements
1. Startup overhead remains predictable and bounded.
2. Backward compatibility for existing MCP tools/CLI commands.
3. Graceful degradation when embedder/vector paths are unavailable.
4. No crash when artifacts are missing.

## Out of Scope
1. Full adaptive routing policy learning.
2. Cross-project federated memory sync.
3. Automatic self-healing workflow retries based on telemetry.

## Success Metrics
1. Reduction in repeated context tokens per comparable task.
2. Increased retrieval hit-rate on multi-step runs.
3. Reduction in divergent outputs between `skill run` and workflow execution.
4. Zero critical regressions in hosted-mcp execution.
5. Observable persona-policy events for unauthorized tool attempts.

## Acceptance Criteria
1. `cargo check -p agent007` passes.
2. `cargo test -p agent007-web --lib` passes.
3. Dashboard run details show retrieval/token/persona policy sections when artifacts exist.
4. CLI + MCP skill runs write retrieval telemetry artifacts.
5. Persona policy warning artifact appears on violation; strict mode blocks execution.
