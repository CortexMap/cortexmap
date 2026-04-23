# LLM Cost Tracking

## Objective

Introduce end-to-end tracking of cost incurred by every LLM call made by CortexMap (embedding generation, RAG summarization, query generation, claim extraction, groundedness judging, rubric judging). Each outbound call to OpenRouter must capture `prompt_tokens` / `completion_tokens` / `total_tokens` from the provider response, convert that into a USD cost using a per-model pricing table, persist a durable audit record of the call (with correlation back to the originating summary/eval/batch), and expose aggregated costs through structured logs and a query API so operators can see spend per region, per eval run, per model, and in total.

The design centers on the single fact that **brainatlas-be is the only service that actually calls OpenRouter** (`brainatlas-be/crates/infra/src/llm.rs:32-48`). All other services (`orch`, `evals-be`) merely drive brainatlas-be via HTTP. Concentrating capture at that choke point keeps the change minimally invasive.

### Assumptions

- OpenRouter responses include a `usage` object with `prompt_tokens`, `completion_tokens`, and `total_tokens` on both `/embeddings` and `/chat/completions` (the code currently drops it — see `brainatlas-be/crates/infra/src/llm.rs:58-110`).
- Pricing is denominated in USD per million tokens with separate input/output rates per model. Sources of truth: OpenRouter's `/models` endpoint, seeded manually at first.
- Correlation granularity required: per-region-summary for ingest/RAG calls, per-eval-run + per-eval-step for eval calls. A free-form `correlation_id` string suffices; no cross-service UUIDs need to be invented.
- Historical (pre-feature) calls do not need backfill; tracking begins at feature rollout.
- Cost is best-effort: if pricing is missing for a model, we record tokens with `cost_usd = NULL` and emit a warning — never fail the underlying LLM call.
- Fetcher-be is out of scope (no LLM traffic); frontend display is out of scope and tracked as follow-up.

## Implementation Plan

### Phase 1 — Domain & Wire Types (foundation, no behavior change)

- [x] Task 1. Add a `Usage` value type in the brainatlas-be domain crate.
  - Location: `brainatlas-be/crates/domain/src/` (new `usage.rs`, re-export from `lib.rs`).
  - Fields: `prompt_tokens: u32`, `completion_tokens: u32`, `total_tokens: u32`.
  - Derive: `Debug`, `Clone`, `Copy`, `Default`, `Serialize`, `Deserialize`, `PartialEq`.
  - Rationale: A pure, serde-capable type that crosses domain → services → infra → rpc-types boundaries without a dependency cycle.

- [x] Task 2. Introduce an `LlmCallOutcome<T>` wrapper (or pair-return convention) in the domain crate.
  - Shape: `{ value: T, usage: Usage, model: String, endpoint: LlmEndpointKind }`.
  - `LlmEndpointKind` enum: `Embedding`, `ChatCompletion`, `ChatCompletionWithTools`.
  - Rationale: Lets every LLM-producing method return both the payload and the observed usage without proliferating tuples in trait signatures.

- [x] Task 3. Add a `CorrelationId` newtype (or `Option<String>`) and thread it through request DTOs.
  - rpc-types updates (`brainatlas-be/crates/rpc-types/src/evals.rs:11-52`): add `correlation_id: Option<String>` to `EmbedRequest`, `ExtractClaimsRequest`, `JudgeGroundednessRequest`, `JudgeRubricRequest`.
  - Proto updates (`proto/llm/brain.proto:86-131`): add optional `string correlation_id` to `ProcessRegionRequest` and `GenerateQueriesRequest`; regenerate prost bindings.
  - Rationale: Provides the join key that links a persisted cost row back to its originating region summary or eval step. Unknown callers simply omit it.

### Phase 2 — Capture `usage` from OpenRouter (infra layer)

- [x] Task 4. Extend OpenRouter response structs in `brainatlas-be/crates/infra/src/llm.rs:58-110` to deserialize `usage`.
  - Add `Usage` (infra-private) with the three token fields; add `usage: Option<Usage>` to both `EmbeddingResponse` and `ChatResponse`.
  - Emit a `tracing::warn!` when `usage` is absent (defensive; some providers omit it).

- [x] Task 5. Change the `EmbeddingGenerator` and `LlmClient` trait return types in `brainatlas-be/crates/services/src/infra.rs:85-122`.
  - `generate_embedding` → returns `LlmCallOutcome<Vec<f32>>`.
  - `summarize_with_tools` → returns `LlmCallOutcome<LlmResponse>`.
  - `generate_queries` → returns `LlmCallOutcome<Vec<String>>` *aggregated across the up-to-three internal iterations* (sum the usage of each inner call).
  - Rationale: Aggregation at the infra boundary preserves semantic equivalence (one logical call in, one usage record out) while still reflecting real cost.

- [x] Task 6. Update `OpenRouterClient` implementations in `brainatlas-be/crates/infra/src/llm.rs:112-469` to produce and return `LlmCallOutcome`.
  - For the iterative `generate_queries` loop (`brainatlas-be/crates/infra/src/llm.rs:260-469`), accumulate token counts across iterations into a single `Usage`.
  - Preserve current log output; add a new structured `tracing::info!("llm.call", endpoint=…, model=…, prompt_tokens=…, completion_tokens=…, total_tokens=…, cost_usd=…, correlation_id=…)` at successful exit of each call.

### Phase 3 — Pricing source of truth

- [x] Task 7. Add a new migration `brainatlas-be/migrations/YYYY-MM-DD-000001-add_llm_pricing/up.sql` that creates table `llm_pricing`.
  - Columns: `id UUID PK`, `model VARCHAR(256) NOT NULL`, `input_price_per_million NUMERIC(12,6) NOT NULL`, `output_price_per_million NUMERIC(12,6) NOT NULL`, `embedding_price_per_million NUMERIC(12,6) NULL`, `currency VARCHAR(8) NOT NULL DEFAULT 'USD'`, `effective_from TIMESTAMPTZ NOT NULL DEFAULT NOW()`, `created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()`.
  - Unique index on `(model, effective_from)`; lookup index on `(model)` — latest row wins.
  - Seed rows for the current default models: `openai/gpt-4o-mini`, `openai/gpt-4o`, `text-embedding-3-small`. Treat the seeded values as replaceable.
  - Add corresponding `down.sql`.

- [x] Task 8. Regenerate `brainatlas-be/crates/infra/src/schema.rs` via `diesel print-schema` and add a Diesel model struct `LlmPricingRow`.

- [x] Task 9. Define an `LlmPricingRepo` port in `brainatlas-be/crates/services/src/infra.rs`.
  - Methods: `async fn latest_for_model(&self, model: &str) -> Result<Option<LlmPricing>, Self::Error>`.
  - Implement it in the infra crate alongside other Postgres adapters.
  - In services, implement an in-memory LRU cache in front of the port (TTL ~5 minutes) to avoid a DB roundtrip per LLM call.
  - Rationale: Pricing changes infrequently; caching keeps hot-path latency unchanged.

### Phase 4 — Persist cost per call

- [x] Task 10. Add a migration `brainatlas-be/migrations/YYYY-MM-DD-000002-add_llm_call_usage/up.sql` creating `llm_call_usage`.
  - Columns: `id UUID PK DEFAULT gen_random_uuid()`, `created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()`, `endpoint VARCHAR(32) NOT NULL` (`embedding`/`chat`/`chat_tools`), `model VARCHAR(256) NOT NULL`, `prompt_tokens INT NOT NULL`, `completion_tokens INT NOT NULL`, `total_tokens INT NOT NULL`, `cost_usd NUMERIC(14,8) NULL`, `correlation_id VARCHAR(128) NULL`, `region_id INT NULL` (FK to `region_mapping`), `summary_id UUID NULL` (FK to `region_summary`), `batch_id UUID NULL` (FK to `region_processing_batches`), `caller_tag VARCHAR(64) NULL` (e.g. `rag_summarize`, `generate_queries`, `judge_rubric`), `request_id VARCHAR(128) NULL`.
  - Indexes on `(created_at)`, `(model, created_at)`, `(correlation_id)`, `(region_id, created_at)`.
  - Add corresponding `down.sql`.

- [x] Task 11. Regenerate Diesel schema and add `NewLlmCallUsage` / `LlmCallUsageRow` models; add a `LlmUsageRepo` port in services with `record(&self, row: NewLlmCallUsage) -> Result<(), Self::Error>` and a query method `aggregate(filter) -> Result<UsageAggregate, Self::Error>` (per-model, per-region, per-correlation).

- [x] Task 12. Implement a thin "cost accounting" helper in the services layer that:
  1. Receives a `LlmCallOutcome<T>` plus contextual metadata (caller tag, correlation id, optional region/summary/batch).
  2. Looks up pricing via the cached `LlmPricingRepo`.
  3. Computes `cost_usd = (prompt_tokens * input_price + completion_tokens * output_price) / 1_000_000` (embedding model uses `embedding_price_per_million` × `total_tokens`).
  4. Persists via `LlmUsageRepo::record`.
  5. Emits the `tracing::info!("llm.call", …)` structured event (deduplicate with Task 6 so the log is emitted exactly once, at the services layer after accounting).
  - Graceful degradation: missing pricing → `cost_usd = NULL` + warn log; persistence failure → error log, do not fail the upstream LLM call.

- [x] Task 13. Wire the accounting helper into every call site in `BrainAtlasLlmService` and `BrainAtlasEmbeddingService` (`brainatlas-be/crates/services/src/llm_service.rs:23-156` and `brainatlas-be/crates/services/src/embedding_service.rs:26-44`).
  - `caller_tag` values: `rag_summarize`, `generate_queries`, `embed`, `extract_claims`, `judge_groundedness`, `judge_rubric`.
  - App-layer methods (`brainatlas-be/crates/app/src/app.rs`) pass through `correlation_id` and `region_id`/`batch_id`/`summary_id` where known.

### Phase 5 — Propagate correlation IDs from upstream services

- [x] Task 14. In `orch`, thread a correlation id into every call to brainatlas.
  - `completion_watcher.rs` (`orch/crates/services/src/completion_watcher.rs:296-541`): set `correlation_id = format!("batch:{batch_id}")` on `ProcessRegionRequest`.
  - `region_management.rs` (`orch/crates/services/src/region_management.rs:241-306`): set `correlation_id = format!("region:{region_id}")` on `GenerateQueriesRequest`.
  - `eval_orchestrator.rs` (`orch/crates/services/src/eval_orchestrator.rs:361-442`): set `correlation_id = format!("eval:{run_id}:{step_id}")` on each of the four `/api/llm/*` POST bodies.
  - Rationale: These strings are sufficient to slice spend per batch/region/eval-run without introducing shared UUIDs.

- [x] Task 15. In brainatlas-be server handlers (`brainatlas-be/crates/server/src/server.rs:150-273`), extract the incoming `correlation_id` and pass it through to app-layer calls. Update `BrainAtlasApi` method signatures in `brainatlas-be/crates/api/src/brainatlas_api.rs:22-175` to accept `correlation_id: Option<String>`.

### Phase 6 — Expose aggregated cost

- [x] Task 16. Add a read-only query endpoint in brainatlas-be: `GET /brainatlas-be/api/llm/usage?since=…&model=…&correlation_id=…`.
  - Returns aggregate `{ total_cost_usd, total_tokens, by_model: [...], by_caller_tag: [...] }`.
  - Backed by `LlmUsageRepo::aggregate`.
  - Register in `brainatlas-be/crates/server/src/server.rs:60-91` router and add a matching method on `BrainAtlasApi`.

- [x] Task 17. Surface cost in the orch public API.
  - Add `total_cost_usd: Option<double>` and `total_tokens: Option<int64>` to `RegionSummary` in `proto/orch/orch.proto:80-85`, computed by joining `region_summary.id` with `llm_call_usage.summary_id` (cross-service read via brainatlas's new usage query endpoint or, if preferred, via a dedicated gRPC method `GetRegionLlmUsage(region_id)`).
  - Wire through `orch/crates/services/` and `orch/crates/api/`.
  - Rationale: The frontend can later render "this summary cost $0.042" without another migration.

- [x] Task 18. Add a second orch endpoint for eval-run cost: `GET /orch/api/evals/runs/{run_id}/cost` returning `{ total_cost_usd, total_tokens, per_endpoint: [...] }` by filtering brainatlas usage where `correlation_id LIKE 'eval:{run_id}:%'`.

### Phase 7 — Observability & ops

- [x] Task 19. Ensure the structured `tracing::info!("llm.call", …)` event (Task 12) includes ALL of: `endpoint`, `model`, `caller_tag`, `prompt_tokens`, `completion_tokens`, `total_tokens`, `cost_usd`, `correlation_id`, `region_id`, `summary_id`, `batch_id`, `latency_ms`.
  - Standardize the event name `llm.call` so log aggregators can pivot on it.

- [x] Task 20. Add a "cost guardrail" configuration knob.
  - New env vars read in `brainatlas-be`: `LLM_COST_DAILY_USD_BUDGET` (optional), `LLM_COST_WARN_THRESHOLD_USD` (optional).
  - Background task (or on-demand check in the accounting helper) that aggregates the last 24h of `llm_call_usage` and emits a `tracing::warn!` (and returns 503 if over hard budget) when exceeded.
  - Defer the 503 behavior behind a feature-flag env var defaulting to off.

### Phase 8 — Tests

- [x] Task 21. Unit tests in `brainatlas-be/crates/infra/` mocking OpenRouter responses with and without `usage` to confirm correct parsing and warn-on-missing behavior.
  - Note: accounting-layer unit tests (CostAccountant) are covered in `brainatlas-be/crates/services/src/cost_accounting.rs` tests; OpenRouter-parsing tests added in `brainatlas-be/crates/infra/src/llm.rs:708-769`.

- [~] Task 22. Integration tests under `tests/` that:
  - Run a full `process_region` against the docker-compose test stack with a stubbed OpenRouter server returning known token counts.
  - Assert `llm_call_usage` rows exist with correct `correlation_id`, `caller_tag`, and `cost_usd`.
  - Assert `GET /brainatlas-be/api/llm/usage` returns matching aggregates.
  - Run an eval cycle and assert per-endpoint rows are recorded with `eval:{run}:{step}` correlation ids.
  - **Deferred**: requires a stubbed OpenRouter HTTP harness and docker-compose fixtures that the current `tests/` directory does not have (the existing `e2e_test.rs` runs against live services). The behaviour under test is covered by `cost_accounting::tests::*` (accounting logic with mock infra) plus `llm::tests::test_*_response_parses_usage*` (OpenRouter wire parsing). Listed here as a follow-up once an OpenRouter mock server lands.

- [x] Task 23. Regression test that asserts an LLM call with missing pricing in `llm_pricing` records tokens with `cost_usd IS NULL` and does not fail the request.
  - Covered: `cost_accounting::tests::record_persists_null_cost_when_pricing_missing` and `finish_swallows_repo_failures` in `brainatlas-be/crates/services/src/cost_accounting.rs`.

### Phase 9 — Rollout

- [x] Task 24. Update `README.md` configuration section with the new env vars (`LLM_COST_DAILY_USD_BUDGET`, etc.) and new endpoints.

- [x] Task 25. Document the seeded prices and a runbook entry for updating `llm_pricing` when OpenRouter changes rates.
  - Covered by the "LLM Cost Tracking" section in README.md, including a sample `INSERT INTO llm_pricing` runbook.

- [x] Task 26. Deploy in order: (1) run both new migrations, (2) deploy updated brainatlas-be (backward compatible — `correlation_id` is optional), (3) deploy updated orch and evals-be.
  - Covered by the "LLM cost tracking rollout note" block in README.md under Production Deployment.

## Verification Criteria

- Every `POST` to OpenRouter produced by brainatlas-be results in exactly one `llm_call_usage` row and exactly one `llm.call` structured tracing event.
- For the three default models (`openai/gpt-4o-mini`, `openai/gpt-4o`, `text-embedding-3-small`), rows contain non-null `cost_usd` and the computed value equals `(prompt_tokens * input_price + completion_tokens * output_price) / 1_000_000` within 1e-8 tolerance.
- An eval-run end-to-end test produces rows whose `correlation_id` matches `eval:<run_uuid>:<step_uuid>` and whose `caller_tag` distinguishes `embed`/`extract_claims`/`judge_groundedness`/`judge_rubric`.
- A region summary end-to-end test produces rows with `correlation_id = batch:<batch_uuid>`, `region_id`, and `caller_tag ∈ {embed, rag_summarize}`.
- `GET /brainatlas-be/api/llm/usage` returns aggregates that equal the sum of matching rows in SQL.
- `GET /orch/api/evals/runs/{run_id}/cost` returns the same total as a direct SQL aggregation.
- When pricing is removed from `llm_pricing`, the call still succeeds and the row has `cost_usd = NULL` plus a WARN log.
- No existing integration test regresses (`./test.sh` passes).
- The change to `EmbeddingGenerator` / `LlmClient` trait return types compiles cleanly in all four services (orch and evals-be do not depend on these traits, so only brainatlas-be needs adjustments there).

## Potential Risks and Mitigations

1. **Provider omits `usage` for some model families (e.g., streaming-only endpoints, future providers routed through OpenRouter).**
   Mitigation: Model `usage` as `Option` at the wire layer; on `None`, persist a row with zero tokens and `cost_usd = NULL` plus a WARN log so missing data is observable and never silently drops a cost event.

2. **Trait signature change ripples wider than expected.**
   Mitigation: Introduce the new return type (`LlmCallOutcome<T>`) via a single shared domain type; the changes are mechanical. Gate the series via `cargo check -p` per crate from `domain` outward (domain → services → infra → app → api → server).

3. **DB write latency per call adds tail latency to summary generation.**
   Mitigation: Fire-and-forget the accounting insert via `tokio::spawn` with bounded concurrency, or batch inserts via a channel with a 250 ms flush; either way, an accounting failure must never block the user request. Prefer the channel approach for ordering guarantees.

4. **Pricing drift — OpenRouter changes rates and our seeded prices become stale.**
   Mitigation: `llm_pricing` stores `effective_from`; the repo selects the latest row for a model. Document the manual update runbook in Task 25. Consider a future nightly sync from OpenRouter's `/models` endpoint as a follow-up.

5. **Correlation id leakage into logs could expose sensitive identifiers.**
   Mitigation: Correlation ids are internal UUIDs (`batch:<uuid>`, `eval:<uuid>:<uuid>`, `region:<int>`) with no PII. Keep them that way; no user input flows into the field.

6. **Cross-service FK reference (`llm_call_usage.summary_id` → `region_summary.id`) creates cascading delete risk if summaries are purged.**
   Mitigation: Use `ON DELETE SET NULL` rather than `CASCADE` so historical cost data is preserved for accounting even if the originating summary is deleted.

7. **Aggregation endpoint scans a large table as usage grows.**
   Mitigation: Indexes on `(created_at)`, `(model, created_at)`, and `(correlation_id)` as part of Task 10. If this proves insufficient, add a materialized daily rollup table in a follow-up.

8. **Race condition: iterative `generate_queries` aggregates usage across 1–3 calls; a mid-loop failure could drop accounting for earlier successful calls.**
   Mitigation: Record each inner iteration as its own `llm_call_usage` row (with the same `correlation_id` and a sequence number in `caller_tag`), rather than aggregating. This also yields more accurate diagnostics.

## Alternative Approaches

1. **Capture at the orch layer instead of brainatlas-be.**
   Trade-offs: Orch already knows the correlation context natively, which would eliminate Phase 5. However, orch does not see provider responses (only brainatlas's reduced response), so it cannot know token counts without brainatlas returning them anyway — which still requires Phases 1–2. Picking this path just shifts the persistence to orch at the cost of an extra wire roundtrip for usage metadata. Rejected: redundant with the chosen approach and splits the concern across services.

2. **Derive cost from a `tracing` subscriber that parses `llm.call` events and writes to the DB.**
   Trade-offs: Zero coupling between LLM code and persistence. But: log-driven persistence is lossy on process crash, harder to test, and mixes observability with source-of-truth data. Rejected for the cost-of-record use case; kept as a supplementary option for downstream BI systems that can consume the JSON log stream.

3. **Use OpenRouter's `/generation` endpoint to retrieve the exact billed cost for each request.**
   Trade-offs: Most accurate (pricing is authoritative). But: doubles the request count, adds latency, and the generation-lookup API is eventually-consistent. Worth considering as an **optional reconciliation** job that backfills `cost_usd` from OpenRouter once a day rather than the primary computation.

4. **Store pricing as `orch_config` rows (key = `price:<model>:input`, value = numeric).**
   Trade-offs: Reuses an existing table (`orch_config` at `orch/migrations/2026-02-14-000001-initial_orch_schema/up.sql:21-37`). But: one-row-per-price is awkward, there's no typed model for it, and pricing logically belongs to brainatlas-be (the service making the calls). Rejected.

5. **Introduce OpenTelemetry + a Prometheus `llm_cost_usd_total` counter instead of (or in addition to) a DB table.**
   Trade-offs: Excellent for real-time dashboards, but metrics alone cannot answer "what did this specific region's summary cost?" because high-cardinality labels (region_id, summary_id) explode Prometheus series. A hybrid is ideal: DB for per-call audit, low-cardinality counters for dashboards. Recommended as a follow-up once the DB table is in place.
