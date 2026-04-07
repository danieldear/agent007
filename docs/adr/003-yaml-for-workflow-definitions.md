# ADR-003: YAML for Workflow Definitions

**Date:** 2026-04-07  
**Status:** Accepted  
**Deciders:** agent007 core team

## Context

Workflows in agent007 are multi-step pipelines where each step specifies an agent persona, a prompt template, output variable name, and optional dependencies on prior steps. Workflows are user-authored files stored in `~/.agent007/workflows/` (global) or `.agent007/workflows/` (project-local).

The key authoring requirements are:

- Steps contain **multi-paragraph prompt text** — sometimes 200–500 words with embedded examples
- Steps reference output variables from other steps (dependency graph)
- Files must be readable and writable by non-Rust developers
- The format must integrate cleanly with `serde` deserialization in Rust

The original design specification described workflow files as TOML. During implementation this was reconsidered.

## Decision

Workflow definitions are **YAML files** (`.yaml` extension) stored in the workflows directory. The schema requires top-level `name`, `description`, and `steps` array fields. Each step requires `id`, `agent`, `prompt`, `output`, and supports optional `depends_on`.

Example step:

```yaml
steps:
  - id: spec
    agent: Planner
    prompt: |
      You are writing a feature spec for: {{task}}

      The spec must include:
      - User stories
      - Acceptance criteria
      - Out of scope items
    output: feature_spec
    depends_on: []
```

## Rationale

- **Multi-line string support**: YAML's literal block scalar (`|`) and folded scalar (`>`) handle multi-paragraph prompt text cleanly. In TOML, multi-line strings require triple-quote syntax (`"""`) and are awkward when prompts contain their own triple-quoted code examples. The `|` syntax is unambiguous and widely understood.
- **CI/CD familiarity**: Workflow files share structural DNA with GitHub Actions and Kubernetes manifests — parallel steps, dependency declarations, named outputs. Users who already write `.github/workflows/*.yml` find the agent007 format immediately recognizable.
- **Editor tooling**: YAML has broad editor support for syntax highlighting, schema validation (via JSON Schema), and linting. TOML support is improving but lags behind YAML in most editors.
- **Serde integration**: `serde_yaml` provides clean deserialization into Rust structs with the same derive macros used everywhere else in the codebase. No special handling required.

## Alternatives Considered

| Format | Reason Not Chosen |
|--------|------------------|
| **TOML** | Original plan. Rejected because TOML multi-line strings are cumbersome for long prompt bodies, and the format is less familiar in the workflow/pipeline domain. TOML remains the format for `agent007.toml` configuration (where its key-value strength shines) |
| **JSON** | Verbose; no support for comments; multi-line strings require escape sequences. Unsuitable for hand-authored prompt-heavy files |
| **Custom DSL** | Would require a parser, documentation, and editor plugin support. The maintenance burden outweighs any expressiveness gain over YAML |
| **HCL (HashiCorp)** | Considered briefly given its use in Terraform. Rejected due to lack of a mature Rust parser and limited familiarity outside the HashiCorp ecosystem |

## Consequences

### Positive

- Prompt text in workflow steps is readable and editable without escaping
- Users familiar with GitHub Actions can author workflows with minimal learning curve
- Steps without `depends_on` are identified as parallelizable by the workflow engine
- Project-local workflows (`.agent007/workflows/`) can be committed to git alongside the code they orchestrate

### Negative / Tradeoffs

- **`serde_yaml` dependency**: Adds a build dependency and the associated YAML parsing code to the binary. YAML parsing is slower than TOML or JSON parsing, though this is not on the hot path
- **Indentation sensitivity**: YAML's significant whitespace is a frequent source of user errors. A misindented step silently changes the document structure. Error messages from `serde_yaml` can be cryptic
- **YAML quirks**: The Norway problem (bare `NO` parsed as boolean `false`), implicit type coercion, and anchor/alias syntax are YAML footguns that can surprise users writing workflow files — though the agent007 schema is narrow enough that most quirks are unlikely to be triggered in practice
- **Not the config format**: Using YAML for workflows and TOML for `agent007.toml` means two serialization formats in the codebase. This is a conscious split — TOML for config, YAML for pipelines — but it adds cognitive surface area

## Related ADRs

- ADR-004 — Hosted-MCP workflow execution (the WorkflowEngine deserializes these YAML files)
- ADR-005 — Skills as Markdown with frontmatter (skills also use YAML, in frontmatter blocks)
