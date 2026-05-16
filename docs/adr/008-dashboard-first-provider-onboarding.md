# ADR-008: Dashboard-First Provider Onboarding

**Date:** 2026-05-16  
**Status:** Accepted  
**Deciders:** agent007 core team

## Context

agent007 currently supports standalone model execution through:

1. `ANTHROPIC_API_KEY`
2. `OPENAI_API_KEY`
3. `[models.ollama]` in `~/.agent007/config.toml`

If none of those are configured, agent007 runs in **hosted-MCP** mode and depends on the host editor/LLM session.

This works, but the current setup model is still operator-heavy:

- users must understand environment variables and config layout
- provider health is not surfaced clearly enough
- local/self-hosted endpoint setup is not guided
- there is no unified place to understand why standalone mode is unavailable

At the same time, agent007 already has a web dashboard and runtime status surface. That dashboard is the natural place to make provider setup more discoverable.

## Decision

Provider onboarding in agent007 will be **dashboard-first**, while preserving file/env compatibility.

This means:

1. The primary user-facing setup flow will live in the web dashboard.
2. Dashboard actions will write or validate the same underlying configuration model (`config.toml`, env-backed provider detection) rather than creating a separate secrets/config system.
3. Manual configuration remains supported and documented for headless, scripted, and advanced setups.
4. OAuth/account-backed login flows are **not** the first slice. The first slice focuses on:
   - status/health visibility
   - guided setup for current providers
   - OpenAI-compatible/local endpoint configuration
   - better error reporting and validation

## Rationale

- **Matches current product shape**: agent007 already has a dashboard; users should not need a separate auth-only CLI surface for the first usability improvement.
- **Reduces duplicate configuration paths**: the dashboard should not invent a second provider model. It should manage the same provider configuration the runtime already reads.
- **Keeps automation intact**: CI, remote boxes, and power users still need env/config-based setup.
- **Safer first implementation**: health checks, config writing, and explicit validation are much smaller and lower-risk than implementing many provider-specific OAuth/device flows.
- **Supports future expansion**: if selected OAuth providers are later added, the dashboard can host them cleanly without invalidating config-based setups.

## Consequences

### Positive

- Users get a visible provider status surface tied to runtime mode.
- Local/self-hosted endpoint setup becomes easier to validate.
- Hosted-MCP vs standalone mode becomes easier to understand.
- The same configuration remains usable from CLI, files, and dashboard.

### Negative / Trade-offs

- agent007 will still lag tools like jcode on multi-provider OAuth breadth in the short term.
- Dashboard-first onboarding increases dependence on the web surface for the best UX.
- Provider-specific OAuth support, if added later, will still require careful credential storage and revocation design.

## First Slice

1. Provider status panel in dashboard
2. Guided validation/setup for:
   - Claude env/API-key path
   - OpenAI/Codex env/API-key path
   - Ollama local endpoint
   - OpenAI-compatible endpoint
3. Health and failure explanations
4. Documentation updates that clearly state:
   - dashboard-first onboarding
   - config/env compatibility remains
   - hosted-MCP remains a first-class mode

## Alternatives Considered

| Alternative | Reason Not Chosen |
|-------------|------------------|
| **CLI-first provider login (`agent007 login --provider ...`)** | Adds a second setup UX before the dashboard/provider-state UX is mature; less aligned with agent007’s existing operator surface |
| **OAuth-first implementation** | Higher complexity, provider-specific maintenance, and secret/session handling burden before basic setup visibility is solved |
| **Keep config/env only** | Lowest implementation cost, but continues the current usability gap and hides runtime/provider problems from normal users |

## Related ADRs

- ADR-002 — MCP stdio transport
- ADR-004 — Hosted-MCP workflow execution mode
- ADR-005 — Skills as Markdown with frontmatter
