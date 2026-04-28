# Design Document: Retrieval-First Reliability Improvements

## Scope
- Bounded RAG warmup indexing
- Shared skill execution path
- Retrieval telemetry artifacts
- Persona MCP tool-policy guardrail
- Dashboard artifact visibility

## Data Models
1. Retrieval telemetry:
   - indexed_docs
   - retrieval_queries
   - retrieval_hits
   - retrieval_hit_rate
   - rag_context_chars
   - vector_hits
   - fallback_hits
   - mock_embedding
2. Persona policy warning:
   - active_persona
   - requested_tool
   - allowed_tools
   - strict_mode
   - message

## Control Flags
- `AGENT007_RAG_WARMUP=0` disables warmup indexing.
- `AGENT007_ENFORCE_PERSONA_TOOLS=1` converts policy from warning to hard block.

## Verification Plan
1. `cargo check -p agent007`
2. `cargo test -p agent007-web --lib`
3. `npm run build` in `crates/web/frontend`
4. Manual run-detail validation in dashboard
