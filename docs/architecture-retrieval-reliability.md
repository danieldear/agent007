# Architecture: Retrieval Reliability + Policy Enforcement

## Module Breakdown
1. `crates/cli/src/commands/run.rs`
   - Builds runtime stack, runs bounded warmup indexing, emits baseline retrieval telemetry.
2. `crates/cli/src/commands/skill.rs`
   - Routes CLI skill execution through shared stack executor.
3. `crates/memory/src/retriever.rs`
   - Returns retrieval context plus stats (`retrieve_with_stats`).
4. `crates/skills/src/executor.rs`
   - Exposes `execute_with_report` with output + metrics.
5. `crates/cli/src/commands/serve.rs`
   - Persists telemetry artifacts and enforces persona tool policy (warn/block).
6. `crates/web/src/api.rs`
   - Exposes telemetry/policy/token artifacts in run detail responses.
7. `crates/web/frontend/src/views/DashboardView.vue`
   - Renders retrieval telemetry, token summary, and policy warnings.

## Data Flow
1. Run/skill starts and builds stack.
2. Warmup indexing scans bounded sources and upserts embeddings.
3. Skill execution retrieves context and stats.
4. Executor returns report.
5. CLI/serve writes artifacts.
6. Web API returns artifact payloads.
7. Dashboard visualizes run-level observability.

## Interface Contracts
1. `Retriever::retrieve_with_stats(query) -> (context, RetrieveStats)`.
2. `SkillExecutor::execute_with_report(skill, args) -> SkillExecutionReport`.
3. Artifacts:
   - `retrieval-telemetry.json`
   - `persona-policy-warning.json`
   - `token-summary.json`
4. Strict persona policy gate toggled by `AGENT007_ENFORCE_PERSONA_TOOLS=1`.
