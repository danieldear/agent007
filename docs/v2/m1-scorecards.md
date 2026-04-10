# M1 Implementation: Run Scorecards, KPI APIs, Regression Harness

This document describes the V2 M1 implementation delivered in this repository.

## What Was Added

1. Canonical run scorecard artifact per run: `run-scorecard.json`
2. Dashboard/API KPI aggregates for success/cost/latency/retries
3. Regression evaluation endpoint against configurable thresholds
4. `crates/p2p` scaffold for upcoming local-first collaboration transport

## Scorecard Lifecycle

Each run now keeps a KPI scorecard updated across run lifecycle events.

```text
create_run
  -> initialize scorecard (running, zeroed counters)
append_event
  -> update tokens/tool metrics
append_note(workflow-step-retry)
  -> increment retry count
finish_run / update_run_status / set_provider
  -> finalize status/latency/cost and persist
```

Primary implementation: `crates/core/src/run_store.rs`

## Scorecard Schema (v1)

Artifact filename: `run-scorecard.json`

Core fields:
- `schema_version`
- `run_id`, `kind`, `mode`, `provider`
- `status`, `completed`, `success`
- `started_at`, `finished_at`, `duration_ms`
- `tokens`, `requests`, `estimated_usd`
- `retry_count`, `tool_calls`, `tool_errors`
- `quality_score`, `updated_at`

## API Additions

### `GET /api/scorecards`
Returns recent run scorecards.

Query params:
- `limit` (optional, default `100`, min `1`, max `500`)

### `GET /api/regression/evaluate`
Evaluates current scorecard KPI summary against thresholds.

Query params:
- `limit` (optional, default `100`)
- `min_success_rate` (optional)
- `max_avg_cost_usd` (optional)
- `max_avg_latency_ms` (optional)
- `max_avg_retries` (optional)

Response fields include:
- `summary`
- `thresholds`
- `passed`
- `violations`

Primary implementation: `crates/web/src/api.rs`

## Usage Examples

List the latest 25 scorecards:

```bash
curl -s "http://localhost:8007/api/scorecards?limit=25" | jq
```

Evaluate regression with custom thresholds:

```bash
curl -s "http://localhost:8007/api/regression/evaluate?limit=100&min_success_rate=0.8&max_avg_cost_usd=2.5&max_avg_latency_ms=90000&max_avg_retries=1.5" | jq
```

Inspect dashboard stats with scorecard KPIs:

```bash
curl -s "http://localhost:8007/api/stats" | jq
```

## Dashboard Metrics Additions

`/api/stats` now includes additive fields:
- `scorecard_run_count`
- `success_rate`
- `avg_cost_usd`
- `avg_latency_ms`
- `total_retries`
- `avg_retries_per_run`
- `recent_scorecards`

Primary implementation: `crates/web/src/metrics.rs`
UI update: `crates/web/frontend/src/views/DashboardView.vue`

## Regression Harness

Implemented in `crates/testing/src/regression.rs`:
- `RegressionThresholds`
- `ScorecardSummary`
- `RegressionEvaluation`
- `summarize_scorecards(...)`
- `evaluate_kpi_regression(...)`

Default threshold profile:
- `min_success_rate = 0.70`
- `max_avg_cost_usd = 5.0`
- `max_avg_latency_ms = 120000`
- `max_avg_retries = 2.0`

## Configuration Guide

1. Runtime defaults require no extra configuration for M1.
2. Regression thresholds can be tuned per request via `GET /api/regression/evaluate` query params.
3. Scorecard/data volume returned by `/api/scorecards` is controlled by `limit` (default `100`, max `500`).
4. Cost estimation uses shared `TOKEN_PRICE_PER_TOKEN_USD` from `agent007-core` to keep core and web math aligned.

## P2P Scaffold (M4 Starter)

New crate: `crates/p2p`

Modules:
- `identity` (peer identity + trust level)
- `discovery` (discovery trait + local stub)
- `announce` (announcement types)
- `service` (no-op service facade)

This is intentionally boundary-only and does not activate network transport in M1.

## Compatibility Notes

- Existing editor/client workflows remain unchanged.
- API changes are additive.
- Legacy runs without scorecards are synthesized by metrics/API paths.

## Known Limitations

1. Historical backfill for old runs is on-demand synthesis, not a dedicated offline migration command.
2. Corrupt legacy artifacts degrade gracefully but should be covered by additional hardened fixture tests.
3. P2P support in M1 is scaffold-only (no active networking, no discovery transport enabled).

## Changelog Entry (V2 M1)

### Added
- Canonical `run-scorecard.json` artifact lifecycle in core run store.
- KPI aggregates for scorecards in dashboard stats.
- `GET /api/scorecards` endpoint for recent scorecards.
- `GET /api/regression/evaluate` endpoint with configurable thresholds.
- Regression summary/evaluation module in `crates/testing`.
- `crates/p2p` scaffold crate for future local-first collaboration.

### Changed
- Unified token-price constant to shared core source for consistent cost calculations across core and web.

### Notes
- All M1 changes are additive and backward-compatible with existing integrations.
