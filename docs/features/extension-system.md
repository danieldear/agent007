# Extension System Architecture

**Status:** Baseline implemented (Phase 1 + major Phase 2 components)  
**Last updated:** 2026-05-03

---

## Problem This Solves

agent007 had strong internal primitives (skills, workflows, memory, personas), but lacked a unified way to bring in external capability sets and activate them consistently.

The extension system now provides a normalized ingestion and install path across multiple ecosystems.

---

## Implemented Ground Truth

| Capability | Status | Notes |
|---|---|---|
| Extension bundle model (`ExtensionBundle`) | ✅ | Canonical structure for skills/tools/workflows/MCP/RAG payloads |
| Compatibility grades (`A/B/C`) | ✅ | Returned by adapters |
| Native local extension adapter | ✅ | `manifest.toml` based |
| GitHub adapter | ✅ | Supports `manifest.toml`, `agent007.json`, discovered files |
| npm MCP adapter | ✅ | Converts package to MCP server registration intent |
| OpenAPI adapter | ✅ | Adapts API spec into importable extension payload |
| Claude marketplace adapter | ✅ | Adapts marketplace metadata into extension payload |
| Extension preview API | ✅ | `POST /api/extensions/preview` |
| Extension install API | ✅ | `POST /api/extensions/install` |
| Installed extensions API | ✅ | `GET /api/extensions/list` |
| Extensions dashboard view | ✅ | Browse/import/installed tabs |
| MCP registry API + dashboard view | ✅ | Full CRUD/connect/approve/tool-list flows |
| RAG source API | ✅ | CRUD + reindex + query |

---

## API Surface (Current)

### Extensions

1. `POST /api/extensions/preview`
2. `POST /api/extensions/install`
3. `GET /api/extensions/list`

### MCP Registry

1. `GET /api/mcp/servers`
2. `POST /api/mcp/servers`
3. `DELETE /api/mcp/servers/{name}`
4. `POST /api/mcp/servers/{name}/connect`
5. `POST /api/mcp/servers/{name}/approve`
6. `GET /api/mcp/servers/{name}/tools`

### RAG Sources

1. `GET /api/rag/sources`
2. `POST /api/rag/sources`
3. `POST /api/rag/sources/{id}/reindex`
4. `DELETE /api/rag/sources/{id}`
5. `GET /api/rag/query`

---

## Bundle Shape

```text
<extension>/
  manifest.toml
  skills/
  tools/
  workflows/
  mcp/
  rag/
  personas/
  hooks/
```

`manifest.toml` carries identity, compatibility, and requirements metadata.

---

## Current Install Model

1. Adapter fetches and normalizes source into an `ExtensionBundle`.
2. Install endpoint writes selected components to `agent007_write_home()`.
3. MCP entries are registered via MCP registry APIs.
4. RAG sources are registered via RAG source APIs.
5. Install metadata is recorded in `extensions/installed.json`.

---

## Security and Trust Boundaries

Implemented protections:

1. Relative-path sanitization for bundle file writes.
2. Parent traversal rejection.
3. Tool-level quarantine/approval model exists in tool registry flows.

Known gap:

1. Full extension-wide quarantine review UI (diff-and-approve before activation) is not yet complete.

---

## Compatibility Grades

1. **Grade A**: Native extension metadata (`manifest.toml` or `agent007.json`) with direct import.
2. **Grade B**: Adapted source (npm MCP, OpenAPI, marketplace conversions).
3. **Grade C**: Metadata/discovery fallback where full structure is unavailable.

---

## Remaining Work

1. Extension-level quarantine + review workflow (not only tool-level).
2. Version/conflict policy for extension upgrades.
3. Optional signed extension manifests for stronger provenance.
4. Public registry/distribution flow (if desired later).
