# ADR 005: RepoIndex v2 as the repo-intelligence store

## Status

Accepted.

## Context

`repo_graph_v1.json` stored repo symbols and edges as one serialized graph. That was easy to inspect, but it forced expensive paths to read the full file, deserialize every node and edge, query in memory, and rewrite the whole artifact after updates. Large repos can therefore spend memory on graph materialization instead of the small answer that the LLM actually needs.

agent007 also has multiple repo-intelligence producers:

```text
tree-sitter     -> fast syntax facts
regex fallback  -> baseline facts when a grammar fails
doc scanner     -> docs -> symbol links
future LSP       -> optional semantic facts/references
```

Those producers should not each own a separate database. They should emit normalized repo facts into one queryable index.

## Decision

Add `RepoIndex v2`, stored at:

```text
.agent007/runtime/repo_index_v2.redb
```

`RepoIndex` is the query source for ETR/MCP/prompt-context paths. It stores typed node and edge records plus lookup tables:

```text
nodes              id -> node
edges              id -> edge
symbol_name        lower_name -> symbol ids
module_name        lower_name -> module ids
edges_from         node id -> edge ids
edges_to           node id -> edge ids
file_nodes         path -> node ids
file_edges         path -> edge ids
```

Runtime query flow becomes:

```text
ETR / MCP / context request
        |
        v
RepoIndexReader
        |
        +-- symbol_lookup: symbol_name/module_name -> nodes
        +-- callers:       symbol -> edges_to -> nodes
        +-- callees:       symbol -> edges_from -> nodes
        +-- context:       bounded symbol + neighbor queries
```

The legacy JSON graph remains only as an explicit compatibility artifact for older graph APIs while query hot paths move to `RepoIndex`. New code should prefer `RepoIndex` and avoid loading or creating `repo_graph_v1.json` unless maintaining backward compatibility.

## Why redb

`redb` is a small embedded Rust database. It gives us transactions, durable local storage, MVCC-style reads, and typed tables without adding a server process. It is a better fit for local symbol/edge lookup than a hosted graph database, and it is simpler to ship than RocksDB/Kuzu/Neo4j-style dependencies.

## Tree-sitter and LSP integration model

Tree-sitter and LSP should not write ad-hoc files. The intended model is:

```text
Extractor
  tree-sitter / fallback / docs / future LSP
        |
        v
Normalized facts
  symbols, imports, calls, docs, references
        |
        v
RepoIndexWriter
        |
        v
RepoIndexReader
  ETR, MCP tools, prompt context, dashboard
```

Tree-sitter is the baseline indexer today. Future LSP integration should be an optional async semantic enrichment producer that adds definitions/references/diagnostics through the same index layer.

## Consequences

- ETR symbol/call/context queries no longer need to materialize the full graph JSON.
- Prompt-context collection reads bounded index results instead of full graph vectors.
- The redb file is treated as a runtime artifact and skipped by repo graph/prompt scanning.
- Missing-index query paths and default project init build `repo_index_v2.redb` directly without emitting legacy JSON.
- Incremental per-file writes can be added behind the same `RepoIndexWriter` shape without changing ETR/MCP callers.
