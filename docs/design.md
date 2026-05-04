# Design Document: Extension + Memory/Learning Runtime Hardening

**Status:** Implemented baseline  
**Last updated:** 2026-05-03

## 1. Scope
This design documents the implemented architecture for:

1. Extension ingestion and installation.
2. MCP registry and RAG source management APIs.
3. Tool-registry safety and execution lifecycle.
4. Runtime learning worker loop and memory observability improvements.

## 2. Core Design Decisions
1. **Adapter-first extension ingestion:** normalize multiple sources into one `ExtensionBundle` model.
2. **Local-first install model:** writes to `agent007_write_home()` with project precedence semantics.
3. **Safety before convenience:** reject path traversal and block unsafe writes outside allowed roots.
4. **Learning runs continuously:** runtime workers are spawned in both `run` and `serve` so optimization does not depend on one command path.
5. **Memory compatibility over breakage:** new key patterns coexist with legacy flattened files.

## 3. Extension Architecture
### 3.1 Adapter Layer
Implemented adapters:
1. `NativeAdapter`
2. `GitHubAdapter`
3. `McpNpmAdapter`
4. `OpenApiAdapter`
5. `ClaudeMarketplaceAdapter`

Each adapter outputs a normalized `ExtensionBundle` with compatibility grade metadata (`A/B/C`).

### 3.2 Extension API Surface
Implemented endpoints:
1. `POST /api/extensions/preview`
2. `POST /api/extensions/install`
3. `GET /api/extensions/list`

### 3.3 Install Semantics
1. Components can be installed selectively (`skills`, `tools`, `workflows`, `mcp`, `rag`).
2. Bundle file paths are sanitized before write.
3. Install metadata is persisted in `extensions/installed.json`.

## 4. MCP + RAG Management
### 4.1 MCP Registry APIs
1. `GET /api/mcp/servers`
2. `POST /api/mcp/servers`
3. `DELETE /api/mcp/servers/{name}`
4. `POST /api/mcp/servers/{name}/connect`
5. `POST /api/mcp/servers/{name}/approve`
6. `GET /api/mcp/servers/{name}/tools`

### 4.2 RAG Source APIs
1. `GET /api/rag/sources`
2. `POST /api/rag/sources`
3. `POST /api/rag/sources/{id}/reindex`
4. `DELETE /api/rag/sources/{id}`
5. `GET /api/rag/query`

## 5. Learning Runtime Design
1. `FeedbackCollector` is initialized with optional `InsightGenerator`.
2. `PromptOptimizer` is initialized when learning is enabled.
3. Runtime workers subscribe to learning events and also run on interval.
4. Worker emits optimizer summaries to project memory (`learning:optimizer_last_pass`).
5. Worker purges expired entries in `learning` and `project` scopes.

## 6. Memory Store Hardening
Implemented in `crates/memory/src/store.rs`:
1. Canonical key splitting for both `:` and `/` separators.
2. Legacy filename fallback for existing entries.
3. On-write migration path from legacy flattened files.
4. Expired entry purge support via `ScopedMemoryStore::purge_expired()`.
5. Enumeration/decay behavior updated to include nested key layouts.

## 7. Observability
New endpoint:
1. `GET /api/memory/{scope}/stats`

Exposed fields include:
1. total key count
2. semantic/procedural/episodic counts
3. average confidence
4. learning-scope skill coverage

## 8. Verification Plan
1. `cargo check --workspace`
2. `cargo test -p agent007-memory`
3. `cargo test -p agent007-learning`
4. `cargo test -p agent007-web api_memory_stats_reports_type_counts_and_learning_skills -- --nocapture`
5. `npm run build` (`crates/web/frontend`)
