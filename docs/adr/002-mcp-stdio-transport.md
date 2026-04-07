# ADR-002: MCP stdio Transport for Editor Integration

**Date:** 2026-04-07  
**Status:** Accepted  
**Deciders:** agent007 core team

## Context

agent007 needs to expose its capabilities (skills, workflows, memory, git tools, context compilation) to AI-assisted editors and coding environments. The target hosts include Claude Code, Cursor, Codex CLI, GitHub Copilot CLI, and Zed. Each of these hosts has its own plugin/extension model, but all of them have converged on supporting at least one common AI-native protocol.

The integration must:

- Work without any network configuration by the end user
- Support a growing set of tools (currently 44) with rich structured inputs and outputs
- Be discoverable — editors should be able to enumerate tools automatically
- Require no persistent process management beyond a single `agent007 serve` invocation

## Decision

Expose agent007 capabilities over the **Model Context Protocol (MCP)** using the **stdio transport**, implemented via the `rmcp` crate. The server is started with `agent007 serve --no-dashboard` (or `agent007 serve` for the web dashboard variant) and communicates over `stdin`/`stdout` in the MCP JSON-RPC framing.

Editor configuration is a single JSON stanza pointing to the `agent007` binary — no ports, no TLS, no auth tokens.

## Rationale

- **Universal editor support**: MCP is the protocol that all major AI editors have adopted. Building against MCP means a single implementation works across Claude Code, Cursor, Zed, GitHub Copilot, and any future MCP-compatible host — no per-editor adapters.
- **Zero network configuration**: stdio transport needs no open port, no firewall exception, and no localhost binding. The editor process spawns `agent007 serve` as a child process and reads/writes its stdio directly.
- **Natural process lifecycle**: The server lives and dies with the editor session. No lingering daemons; no stale state from a previous session accidentally leaking into a new one.
- **44 tools, one protocol**: All agent007 capabilities — `agent007_run`, `agent007_skill_run`, `agent007_workflow_start`, `agent007_memory_read`, `agent007_git_commit`, etc. — are registered as MCP tools with JSON Schema definitions. The host editor can invoke any of them without agent007-specific knowledge.
- **Hosted-MCP pattern emerged naturally**: Because the host LLM already has an active MCP connection to agent007, it can orchestrate multi-step workflows by calling `workflow_next` and `workflow_submit_step` in a loop — no separate orchestration channel needed (see ADR-004).

## Alternatives Considered

| Alternative | Reason Not Chosen |
|-------------|------------------|
| **LSP (Language Server Protocol)** | LSP is well-understood for code intelligence but is designed around document/diagnostic operations. It lacks a standard mechanism for arbitrary tool invocation. agent007 does implement an LSP server (`agent007 serve-lsp --stdio`) as a secondary interface for editors that require it, but LSP is not the primary integration path |
| **HTTP REST API** | Would require the user to manage a port, potentially deal with auth, and manually configure each editor. The web dashboard (`http://localhost:8007`) uses HTTP internally, but as a monitoring UI — not as the primary editor integration surface |
| **WebSocket / SSE transport** | MCP supports SSE as an alternative transport. Rejected for the primary integration because it still requires a port and a running HTTP server, adding configuration overhead with no benefit for local-only use |
| **Custom protocol** | Rejected outright. A bespoke protocol would require per-editor adapters and would not benefit from the MCP ecosystem (tool discovery, schema validation, host-side rendering of tool results) |

## Consequences

### Positive

- A single `agent007 serve` command integrates with every MCP-compatible editor
- Tool schema is self-describing — editors can present agent007 tools in their UI without custom code
- The MCP connection carries both tool calls and their results, making the full execution trace observable in editor logs
- New tools can be added to the server and are immediately available to all connected editors after restart

### Negative / Tradeoffs

- **Protocol evolution dependency**: agent007 is coupled to the MCP specification. Breaking changes in MCP (schema format, transport framing) require updates to `rmcp` and potentially to agent007's tool definitions
- **44-tool surface**: A large tool count can overwhelm editors that display all MCP tools inline. Some editors impose tool limits that may require agent007 to support tool filtering in future
- **stdio serialization overhead**: All tool inputs and outputs are JSON-serialized over stdio. Large payloads (e.g., full file diffs passed to a skill) are transmitted as strings rather than shared memory — acceptable at current scale but worth monitoring
- **No server-initiated messages**: stdio MCP is request/response only. The server cannot push notifications to the editor (e.g., a background task completing). Polling via `workflow_status` is the current workaround

## Related ADRs

- ADR-001 — Rust as implementation language (MCP server built on `rmcp` + `tokio`)
- ADR-004 — Hosted-MCP workflow execution (depends on this MCP connection for step orchestration)
