# agent007.json Extension Spec

Any GitHub repository can become a Grade-A agent007 extension by adding `agent007.json` to its root.

## Schema

```json
{
  "agent007": {
    "name": "my-extension",           // required: unique slug
    "version": "1.0.0",              // required: semver
    "description": "...",            // optional
    "skills": ["skills/"],           // optional: dirs/files to import as skills
    "tools": ["tools/"],             // optional: dirs to import as tools
    "workflows": ["workflows/"],     // optional: workflow YAML files
    "mcp": {                         // optional: MCP server to register
      "command": "npx",
      "args": ["-y", "my-mcp-pkg"]
    },
    "rag": [                         // optional: knowledge sources to index
      { "name": "docs", "kind": "url", "source_ref": "https://..." }
    ]
  }
}
```

## How agent007 imports it

1. GitHubAdapter fetches `agent007.json` from the repo root
2. Parses skills/tools/workflows paths relative to repo root
3. Fetches each file via raw.githubusercontent.com
4. Registers MCP server if present
5. Queues RAG sources for indexing if present
6. Installs with Grade A (native convention)

## Compat grades

- **Grade A**: Has `manifest.toml` OR `agent007.json` — full import
- **Grade B**: npm MCP package, OpenAPI spec, Claude marketplace skill — adapted import
- **Grade C**: No recognized format — metadata only, manual completion required
