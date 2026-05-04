# PRD: Extension Platform + Memory/Learning Reliability Hardening

**Status:** Implemented (current baseline)  
**Last updated:** 2026-05-03

## 1. Executive Summary
This PRD captures the shipped product slice that makes agent007 materially more useful than plain prompt-only runs:

1. Extension import and install from multiple sources.
2. First-class MCP server and RAG source management APIs.
3. Tool registry with search/import/test/approval lifecycle.
4. Memory and learning runtime hardening so learning loops run continuously and memory is observable.

## 2. Target Users
1. Solo developers using Codex/Claude/Cursor with agent007.
2. Teams standardizing reusable skills, tools, workflows, and MCP servers.
3. Maintainers who need measurable reliability and low operational drift.

## 3. User Stories
1. As a user, I can import capabilities from local paths, npm MCP packages, GitHub repos, OpenAPI specs, and Claude marketplace-like sources.
2. As a user, I can manage MCP servers and RAG sources via API/dashboard without hand-editing config files.
3. As a user, I can register deterministic tools once and reuse them safely.
4. As a maintainer, I can see memory quality signals and trust that learning loops execute in both `run` and `serve` modes.

## 4. Functional Requirements
1. Extension adapters and bundle model (`A/B/C` compatibility grades).
2. Extension APIs:
   - `POST /api/extensions/preview`
   - `POST /api/extensions/install`
   - `GET /api/extensions/list`
3. MCP registry APIs:
   - `GET|POST|DELETE /api/mcp/servers...`
   - `POST /api/mcp/servers/{name}/connect`
   - `POST /api/mcp/servers/{name}/approve`
4. RAG source APIs:
   - `GET|POST|DELETE /api/rag/sources...`
   - `POST /api/rag/sources/{id}/reindex`
   - `GET /api/rag/query`
5. Tool registry APIs for discovery/search/import/create/test/approve.
6. Runtime learning workers in CLI `run` and `serve` paths.
7. Memory compatibility for `:` and `/` key styles with legacy fallback behavior.
8. Memory observability endpoint: `GET /api/memory/{scope}/stats`.

## 5. Non-Functional Requirements
1. Backward compatibility with legacy memory key files.
2. Deterministic and bounded tool-test execution.
3. No unsafe path traversal during extension install/import.
4. Feature parity across hosted-MCP and local CLI execution paths where applicable.

## 6. Out of Scope (for this slice)
1. Public hosted extension marketplace service.
2. Distributed package version solver.
3. Full autonomous policy learning with zero human guardrails.

## 7. Acceptance Criteria
1. `cargo check --workspace` passes.
2. `cargo test -p agent007-memory` passes.
3. `cargo test -p agent007-learning` passes.
4. `cargo test -p agent007-web api_memory_stats_reports_type_counts_and_learning_skills -- --nocapture` passes.
5. `npm run build` passes in `crates/web/frontend`.
6. Dashboard can surface extension, tools, MCP, and memory stats flows using current APIs.

## 8. Success Metrics
1. Fewer repeated manual setup prompts per project.
2. Higher reuse of deterministic local tools.
3. Better memory transparency (key counts/type mix/confidence visibility).
4. Reduced silent learning drift due to always-on runtime workers.
