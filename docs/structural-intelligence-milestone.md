# M6 — Repo-Native Structural Intelligence and Graph Retrieval

## Goal
Make the repository itself the default first-class context source in agent007 by adding a structural repo graph, graph-aware retrieval, incremental updates, and ETR query surfaces that improve precision and reduce blind file reads.

## Non-Goals
1. Replace the current memory store with a graph database.
2. Depend on Doxygen or any single external documentation generator.
3. Build a giant multi-modal enterprise graph in the first slice.
4. Force every retrieval path to go through graph traversal, even when a direct lookup is cheaper.

## Why This Milestone Exists
agent007 already has:
- `RepoBrain` for compact project summaries
- `MemoryStore` for semantic/procedural/episodic notes
- vector-backed retrieval with keyword fallback
- ETR tools for deterministic extraction and workflow inspection

What it lacks is a durable **structural model of the repo**:
- call graph / caller graph
- usage / reference graph
- import / include / dependency graph
- symbol ↔ file ↔ doc linkage
- incremental structural refresh when the repo changes

Without that layer, agent007 still behaves too much like a raw LLM with better tooling instead of a runtime that can reason from a living model of the codebase.

## Core Architecture

```ascii
repo intelligence stack
├─ layer 1: structural repo graph
│  ├─ files
│  ├─ symbols
│  ├─ imports/includes
│  ├─ calls
│  ├─ usages/references
│  └─ doc links
├─ layer 2: semantic repo retrieval
│  ├─ code/docs/artifact chunks
│  └─ vector + keyword retrieval
├─ layer 3: project memory
│  ├─ run outputs
│  ├─ decisions
│  ├─ notes
│  └─ reusable summaries
└─ layer 4: ETR + workflows
   ├─ graph queries
   ├─ semantic retrieval
   └─ context assembly
```

## Design Principles
1. **Repo graph is not memory** — the graph models what exists and how it connects; memory records what happened and what was learned.
2. **Graph retrieval is additive** — vector retrieval, repo brain, and scoped memory remain valid and should compose rather than be replaced.
3. **Incremental before exhaustive** — patch changed files and their neighboring graph edges instead of rebuilding everything.
4. **Deterministic extraction first** — prefer structural extraction from parsers, tree-sitter, LSP, and indexers before LLM inference.
5. **Graceful degradation** — repo intelligence must still provide a baseline graph without tree-sitter or LSP; semantic enrichment is additive, not required for basic operation.
6. **ETR-first access** — graph queries should be consumable as low-noise ETR tools before broad shell usage.

## Workstreams

### W1 — Structural Repo Graph v1
Build a durable repo graph that captures code and documentation structure at init time.

**Scope**
- candidate crates/files:
  - `crates/core/`
  - `crates/memory/`
  - new candidate crate: `crates/repo-graph/`
  - `crates/cli/src/commands/init.rs`
  - `crates/cli/src/commands/run.rs`

**Deliverables**
1. Graph schema v1:
   - node kinds: file, symbol, module, doc, workflow, skill, persona
   - edge kinds: defines, imports, calls, references, documents, uses
2. Initial repo scan:
   - detect structural units from supported ecosystems
   - persist graph artifact under project-local agent home
3. Structural snapshot metadata:
   - graph version
   - build time
   - indexed roots
   - supported extractor coverage

**Acceptance**
- a fresh project can build a structural graph at init or first context compile
- graph artifact can answer file/symbol relationship questions without LLM inference
- unsupported files fail soft without breaking the rest of the graph

### W2 — Incremental Graph Refresh
Keep structural intelligence current as the repo evolves.

**Scope**
- candidate crates/files:
  - `crates/cli/src/commands/serve.rs`
  - candidate graph storage/update modules
  - session/runtime event plumbing

**Deliverables**
1. Change detection:
   - file add/update/delete
   - dependency-aware re-index boundaries
2. Partial rebuild path:
   - patch affected nodes
   - patch neighboring edges
   - preserve unaffected graph state
3. Freshness markers:
   - last refreshed timestamp
   - stale extractor segments

**Acceptance**
- modifying a file does not require full graph rebuild
- stale graph regions are visible
- refresh cost scales with changed surface area, not total repo size

### W3 — Repo RAG as Default Corpus
Treat the repo as the primary retrieval corpus instead of an implicit afterthought.

**Scope**
- candidate crates/files:
  - `crates/memory/src/retriever.rs`
  - `crates/cli/src/commands/run.rs`
  - `crates/core/src/context.rs`
  - candidate context-assembly helpers

**Deliverables**
1. Hybrid retrieval policy:
   - direct symbol/file hits
   - semantic vector hits
   - graph expansion around seed hits
2. Context assembly contract:
   - minimal relevant bundle
   - file excerpts + graph paths + related notes
3. Repo-first defaults:
   - context compile prefers repo graph + repo RAG before broad memory search

**Acceptance**
- source code, docs, and local artifacts are indexed as the default retrieval corpus
- context assembly is smaller and more targeted than raw top-k chunk stuffing
- answers can cite both semantic hits and structural links

### W4 — Memory Integration Without Conflation
Make memory consume repo-intelligence outputs without turning the graph into a generic note store.

**Scope**
- candidate crates/files:
  - `crates/memory/src/store.rs`
  - `crates/core/src/context.rs`
  - `crates/cli/src/commands/run.rs`
  - memory-related dashboard surfaces

**Deliverables**
1. Memory ingestion rules:
   - run outputs
   - decisions
   - reusable summaries
   - graph-derived impact summaries
2. Cross-linking:
   - memory entries can point to graph nodes/paths
   - graph nodes can reference memory keys where relevant
3. Separation rules:
   - repo graph stays structural
   - memory stays episodic/procedural/semantic

**Acceptance**
- memory items can reference structural evidence
- graph updates do not rewrite unrelated memory
- retrieval can combine graph evidence with prior run knowledge

### W5 — ETR Graph Control and Query Surface
Expose structural intelligence through deterministic tools first, including both graph build/update controls and graph query operations.

**Scope**
- candidate crates/files:
  - `crates/etr/src/l1/`
  - `crates/etr/src/l1/mod.rs`
  - tool docs and manifests

**Deliverables**
1. Candidate ETR control tools:
   - `etr.graph_build`
   - `etr.graph_refresh`
   - `etr.graph_refresh_paths`
   - `etr.graph_status`
   - `etr.graph_compact`
2. Candidate ETR query tools:
   - `etr.symbol_lookup`
   - `etr.callers`
   - `etr.callees`
   - `etr.usage_graph`
   - `etr.dep_path`
   - `etr.doc_links`
   - `etr.impact_radius`
   - `etr.context_bundle`
3. Stable schemas:
   - graph node IDs
   - edge path output
   - confidence/freshness markers
4. Fallback rules:
   - no graph present
   - partial extractor coverage

**Acceptance**
- LLMs, workflows, and runtime hooks can trigger graph build or partial refresh through deterministic ETR calls
- common structural questions can be answered via ETR without shell parsing
- tool outputs are compact and workflow-friendly
- workflows and personas can prefer graph-aware ETR calls by default

### W6 — Dashboard and Operator Visibility
Make the structural layer inspectable instead of invisible.

**Scope**
- candidate crates/files:
  - `crates/web/src/api.rs`
  - `crates/web/frontend/src/views/`
  - `crates/web/frontend/src/components/`

**Deliverables**
1. Repo intelligence status card:
   - graph ready / partial / stale
   - indexed roots
   - refresh age
2. Compact graph inspection:
   - node lookup
   - path preview
   - impact preview
3. Context provenance UI:
   - semantic hit
   - graph hop
   - memory note

**Acceptance**
- users can tell whether repo intelligence is healthy
- graph-backed context is inspectable in the dashboard
- stale structural state is visible before it causes bad answers

### W7 — LSP Semantic Overlay
Use LSP as a semantic enrichment and validation layer on top of the base repo graph.

**Scope**
- candidate crates/files:
  - `crates/lsp-client/`
  - `crates/core/src/repo_graph.rs`
  - `crates/memory/src/retriever.rs`
  - `crates/etr/src/l1/`

**Deliverables**
1. Symbol enrichment:
   - definition/hover/type data where available
   - workspace symbol resolution
   - implementation and reference resolution
2. Semantic relationship overlays:
   - call hierarchy from LSP when supported
   - diagnostics attached to graph nodes/files
   - rename/impact preview support
3. Retrieval fusion:
   - graph + vector + LSP evidence ranking
   - provenance markers that distinguish structural vs semantic evidence
4. Fallback rules:
   - if LSP is absent or unhealthy, the baseline graph still works
   - language-by-language capability reporting

**Acceptance**
- LSP-capable repos gain more precise symbol/reference/call edges without breaking fallback behavior
- graph-backed retrieval can surface semantic provenance
- workflows can request semantic validation before risky edits

### W8 — Dependency Readiness and One-Stop Onboarding
Make missing structural dependencies visible and easy to install without silently mutating the machine.

**Scope**
- candidate crates/files:
  - `crates/cli/src/commands/init.rs`
  - `crates/web/src/api.rs`
  - `crates/web/frontend/src/views/`
  - `crates/lsp-client/`

**Deliverables**
1. Capability detection at init/startup:
   - detect repo languages
   - detect configured/available LSP servers
   - detect tree-sitter parser coverage
   - record readiness artifact under project-local agent home
2. Readiness model:
   - `baseline_ready` (graph works now)
   - `semantic_enrichment_missing` (LSP/tree-sitter absent)
   - `installable` recommendations by language and platform
3. Operator UX:
   - dashboard cards for missing LSP/tree-sitter coverage
   - click-to-copy install command or click-to-run with explicit approval
   - no silent auto-install by default
4. Policy and safety:
   - project chooses whether installs are allowed
   - every install action is explicit, logged, and reversible where possible

**Acceptance**
- users can tell exactly why semantic enrichment is partial
- init gives a one-stop readiness summary instead of failing silently
- dashboard can guide installation of missing enrichers by repo language

## Suggested Execution Order
1. **W1 Structural Repo Graph v1**
2. **W5 ETR Graph Query Surface**
3. **W3 Repo RAG as Default Corpus**
4. **W2 Incremental Graph Refresh**
5. **W4 Memory Integration Without Conflation**
6. **W6 Dashboard and Operator Visibility**
7. **W7 LSP Semantic Overlay**
8. **W8 Dependency Readiness and One-Stop Onboarding**

## Recommended First Slice
Start with **W1 + the smallest part of W5**.

**Why first**
- it creates the foundational artifact every later workstream depends on
- it is testable without changing the whole memory pipeline
- it immediately enables deterministic value via ETR graph queries

**Minimal shippable outcome**
- build a structural graph for a repo at init or first explicit command
- support nodes/edges for files, symbols, imports, and calls
- expose one or two query tools such as `etr.symbol_lookup` and `etr.callers`

## Acceptance Criteria for the Milestone
1. New projects can build and persist a structural repo graph without external generators.
2. The baseline graph works without tree-sitter or LSP.
3. When semantic enrichers are available, agent007 can fuse structural, vector, and LSP evidence.
4. Missing enrichers are surfaced clearly in init and dashboard readiness UI.
5. Users can install recommended enrichers through explicit, auditable actions rather than silent background mutation.
6. Structural queries (callers, callees, usage paths, impact radius) remain available through ETR.
7. Memory can reference graph-derived evidence without becoming the graph itself.

## Precise Product Stance
```ascii
repo intelligence onboarding
├─ baseline graph
│  └─ must work out of the box
├─ semantic enrichment
│  ├─ tree-sitter = better syntax coverage
│  └─ LSP = semantic resolution and diagnostics
└─ install behavior
   ├─ detect automatically
   ├─ recommend automatically
   ├─ allow one-click install with approval
   └─ do NOT auto-install silently by default
```

This keeps agent007 a one-stop solution without making unexpected system-level changes behind the user's back.

## Likely Docs to Update
- `docs/milestones.md`
- `docs/architecture.md`
- `docs/configuration.md`
- `docs/features/tool-registry.md`
- `docs/runtime-and-tui-milestone.md` (cross-reference only if needed)
