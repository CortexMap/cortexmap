# Targeted Re-run of Evals (per-summary, per-metric, or bulk)

## Objective

Add a first-class ability for operators to **re-run a specific eval metric (or the full eval suite) on either a single summary or all summaries**, including the case where the eval *definition* itself has changed to a new version.

Concretely, after this feature lands, a user must be able to:

1. Re-score one metric (e.g. `rubric_relevance`) on one summary, forcing a fresh compute even if the score is cached.
2. Re-score one metric on every summary in the corpus.
3. Re-score all 15 metrics on a single summary.
4. Re-score all 15 metrics on every summary.
5. Do each of the above under **either** the currently-configured `eval_version` (same-version rescore, invalidates cache rows) **or** a *new* `eval_version` (publishes a definition bump and rescores fresh), without losing the history under the old version.
6. Trigger each of the above from `orch`'s HTTP API so the existing concurrency-controlled fan-out (`buffer_unordered(eval_orchestrator_concurrency)`) is reused.
7. Have each re-run interact correctly with future versions of evals (new metrics, new prompts) without another wire-format change.

The plan deliberately introduces a **per-metric version** column so that bumping one metric's prompt does not waste LLM calls on the other 14 metrics, closing the open question flagged in `plans/2026-04-19-evals-architecture-v1.md:287`.

---

## Current State (summary of research)

Key findings drawn from `evals-be`, `orch`, and `atlas`:

- Cache is a UNIQUE index `(summary_hash, metric, eval_version)` on `eval_scores` (`evals-be/migrations/2026-04-19-000001-create_eval_scores/up.sql:24-25`). The only writer is `INSERT ... ON CONFLICT DO NOTHING` in `evals-be/crates/infra/src/pg.rs:181-193`, so the cache is effectively immutable per tuple.
- `POST /evals-be/api/evals/score/init` accepts an optional `eval_version` override (`evals-be/crates/rpc-types/src/lib.rs:14-20`) and is idempotent w.r.t. `eval_run_state` (`evals-be/crates/app/src/app.rs:143-159`), but always read-throughs the cache (`evals-be/crates/services/src/cache.rs:33-80`).
- `eval_runs` is unique on `(summary_id, eval_version)` (`evals-be/migrations/2026-04-19-000001-create_eval_scores/up.sql:51-52`), upserted by `upsert_run` (`evals-be/crates/infra/src/pg.rs:389-443`).
- "Unscored" detection uses a LEFT JOIN on `eval_runs` (`evals-be/crates/infra/src/pg.rs:462-474`), driven by orch's background loop (`orch/crates/services/src/eval_orchestrator.rs:228-289`), gated by `orch_config.eval_orchestrator_enabled` (`orch/migrations/2026-04-19-000001-add_eval_config/up.sql:6`).
- orch exposes only read-only eval proxies today (`orch/crates/server/src/server.rs:88-89`).
- The frontend renders scores (`atlas/src/components/detail/RegionDetail.tsx:488, 562-641`) but has no trigger UI.
- The metric registry is `EvalMetric` (`evals-be/crates/domain/src/evals.rs:66-91`), 15 variants, hard-coded wiring in `run_eval.rs` and `state_machine.rs`.
- `region_summary` has `is_active` and `batch_id` columns already (`evals-be/crates/infra/src/schema.rs:60-76`), not currently enforced by `get_summary` (`evals-be/crates/infra/src/pg.rs:214-221`).

**Gaps for targeted re-run**:

- No delete/invalidate primitive on `eval_scores` or `eval_runs`.
- No endpoint shaped as "re-run eval X on summary Y" or "re-run eval X on all summaries".
- No per-metric version, so a single-metric prompt change cannot be invalidated without wiping everything.
- No orch write-shaped eval endpoint; no frontend button.
- No test coverage for invalidation.

---

## Key Assumptions

These are made explicit so they can be revisited before implementation:

1. **Invalidation semantics are operator-facing and intentional.** Re-run calls are an admin action, not an end-user action; we will gate them behind an authenticated / dev-only route (same scope as `orch/crates/server/src/dev_stats.html`). No per-user throttling in v1.
2. **`eval_version` remains a global string** identifying a coherent snapshot of every metric's definition. In addition, each metric row gets its own `metric_version` string so a single metric can be revved without touching the global version.
3. **Re-run "on all summaries" means "all active summaries"** (`region_summary.is_active = true`). Inactive / superseded rows are not re-scored unless the caller passes an explicit flag.
4. **Orch remains the only concurrency governor.** evals-be re-run endpoints do not spawn background work; they either (a) delete cache rows and return, letting the orch poll-loop pick them up, or (b) enqueue an in-memory work list that the existing `run_cycle` consumes.
5. **Backwards compatibility is preserved.** The existing `POST /evals-be/api/evals/score/init` behavior is unchanged; new endpoints are additive.
6. **No protobuf changes are required.** All eval wires are already JSON over HTTP (`proto/` has zero eval references); new endpoints stay on JSON.
7. **Dry-run and audit.** Every re-run returns the list of `(summary_id, metric)` rows it invalidated, for logging and optional UI feedback. No separate audit table is introduced in v1.
8. **The citation-metric work-in-progress** (`plans/2026-04-20-citation-correctness-evals-v1.md`) lands independently; this plan assumes `EvalMetric::all()` may grow. Nothing here hard-codes the current 15-metric count.

---

## Implementation Plan

### Phase 1 — Data model: introduce `metric_version` and explicit invalidation

- [ ] **Task 1. Add `metric_version` column to `eval_scores` and `eval_runs`.**
  Create a new Diesel migration under `evals-be/migrations/` (e.g. `2026-04-21-000001-add_metric_version`) that:
  - Adds `metric_version VARCHAR(16) NOT NULL DEFAULT 'v1'` to both tables.
  - Drops and recreates `ix_eval_scores_cache` as UNIQUE `(summary_hash, metric, eval_version, metric_version)`.
  - Leaves `ix_eval_runs_unique` at `(summary_id, eval_version)` (runs remain per-version, not per-metric-version; a run aggregates mixed metric versions). Alternatively extend to `(summary_id, eval_version, metric_version)` if we prefer a run per metric-version bump — see Alternative Approaches.
  - Back-fills existing rows with `'v1'`.
  **Rationale**: enables bumping a single metric's definition without wasting compute on the other 14 cached metrics, and underpins "re-run only `rubric_relevance`" semantics.

- [ ] **Task 2. Expose a per-metric version registry in code.**
  Extend `EvalMetric` in `evals-be/crates/domain/src/evals.rs:66-91` with a `fn version(self) -&gt; &'static str` returning a per-variant constant (e.g. `"v1"` for all today). Establish the convention that any change to a metric's structural code or prompt *must* bump its version constant in the same PR. Add a unit test asserting every variant has a non-empty version string.
  **Rationale**: makes the per-metric version a compile-time artefact alongside the metric, closing the footgun of forgetting to bump the global `eval_version`.

- [ ] **Task 3. Thread `metric_version` through the cache path.**
  Update `NewEvalScore` (`evals-be/crates/domain/src/evals.rs:26-35`), the diesel insertable in `evals-be/crates/infra/src/pg.rs:181-193`, and `score_with_cache` (`evals-be/crates/services/src/cache.rs:33-80`) so that the cache lookup and write both include `metric_version`. Every call site in `evals-be/crates/app/src/run_eval.rs:30-93` and `evals-be/crates/services/src/state_machine.rs` supplies the metric's registered version.
  **Rationale**: without this, the new column is unused and the Phase-1 migration is dormant.

- [ ] **Task 4. Add database methods for targeted deletion.**
  In `evals-be/crates/services/src/lib.rs` (the `EvalsDatabase` trait) and its impl at `evals-be/crates/infra/src/pg.rs`, add:
  - `delete_scores(&amp;self, summary_id: Option&lt;Uuid&gt;, metric: Option&lt;String&gt;, eval_version: String, metric_version: Option&lt;String&gt;) -&gt; Result&lt;Vec&lt;(Uuid, String)&gt;, _&gt;` returning the `(summary_id, metric)` pairs deleted.
  - `delete_runs(&amp;self, summary_id: Option&lt;Uuid&gt;, eval_version: String) -&gt; Result&lt;usize, _&gt;`.
  Both are `DELETE ... RETURNING` queries; the `Option` args control breadth (None = all active summaries).
  **Rationale**: gives the re-run endpoint a single atomic primitive for cache invalidation, and returns an audit trail to the caller.

- [ ] **Task 5. Filter on `region_summary.is_active` when resolving summaries for bulk operations.**
  In `evals-be/crates/infra/src/pg.rs:214-221` (`get_summary`) keep the plain `.find` for single-summary lookups, but add a new `list_active_summary_ids(eval_version) -&gt; Vec&lt;Uuid&gt;` method that filters `WHERE is_active = TRUE AND summary IS NOT NULL`. Update `list_unscored_summary_ids` (`pg.rs:462-474`) to share the same filter.
  **Rationale**: bulk re-runs must not waste compute on superseded summaries.

### Phase 2 — evals-be: re-run endpoints

- [ ] **Task 6. Design the re-run wire contract.**
  Add the following JSON types to `evals-be/crates/rpc-types/src/lib.rs`:
  - `RerunScope`: `tagged enum { Summary { summary_id: Uuid }, AllSummaries }`.
  - `RerunSelector`: `tagged enum { AllMetrics, Metric { metric: String } }`.
  - `RerunRequest { scope: RerunScope, selector: RerunSelector, eval_version: Option&lt;String&gt;, trigger: bool }` — `trigger = true` means "kick orch to start work now"; `false` means "just invalidate; let the poll loop pick it up".
  - `RerunResponse { eval_version: String, invalidated: Vec&lt;(Uuid, String)&gt;, enqueued: Vec&lt;Uuid&gt; }`.
  **Rationale**: one unified shape covers the 2×2 matrix (per-summary vs. bulk) × (per-metric vs. all metrics) with room for a future `Metrics { metrics: Vec&lt;String&gt; }` selector.

- [ ] **Task 7. Add the re-run app method.**
  In `evals-be/crates/app/src/app.rs` add `EvalsApp::rerun(req: RerunRequest) -&gt; Result&lt;RerunResponse, AppError&lt;E&gt;&gt;` that:
  1. Resolves `eval_version` (request override or `ConfigKey::EvalVersion`).
  2. Calls `delete_scores` + `delete_runs` with the matching scope/selector.
  3. If `trigger = true` and `scope = Summary`, invokes the existing `init_score` pipeline synchronously (returning its first `NextAction` under a new field of `RerunResponse`, or by calling `drive_one` — see Phase 3).
  4. If `trigger = true` and `scope = AllSummaries`, returns the list of `summary_id`s to be processed; orch's poll loop, already running, will pick them up next tick (they are now unscored for the given `eval_version`).
  **Rationale**: separates deterministic invalidation (always runs) from fan-out triggering (optional), keeping the app layer side-effect-free and easy to test.

- [ ] **Task 8. Add the `EvalsApi` trait method and HTTP route.**
  Extend `EvalsApi` at `evals-be/crates/api/src/api.rs:15-40` with a `rerun` method, implement the forward in `impl EvalsApi for Evals`, and mount `POST /evals-be/api/evals/rerun` in `evals-be/crates/server/src/server.rs:57-77`.
  **Rationale**: follows the existing thin-facade pattern so error-handling and instrumentation are uniform with `init_score`/`step_score`.

- [ ] **Task 9. Keep `POST /evals-be/api/evals/score/init` backward-compatible, but add a `force: bool` flag.**
  Extend `InitScoreRequest` (`evals-be/crates/rpc-types/src/lib.rs:14-20`) with `#[serde(default)] force: bool`. When `true`, `init_score` deletes the `(summary_id, eval_version)` score + run rows before proceeding — giving CLI callers a one-shot "force rescore this one summary" without the broader re-run plumbing.
  **Rationale**: provides a minimal-blast-radius re-run for scripts and for tests, while the richer `/rerun` endpoint handles the UI/bulk path.

### Phase 3 — orch: write-shaped eval endpoints and bulk fan-out

- [ ] **Task 10. Add an `EvalOrchestration::rerun` trait method.**
  In `orch/crates/app/src/services.rs:258-280` extend the trait with an async `rerun(req: RerunRequest) -&gt; RerunResponse`. The implementation at `orch/crates/services/src/services.rs:436-455` delegates to `EvalOrchestrator::rerun`.
  **Rationale**: keeps the service/app split consistent with the existing `get_eval_status` / `get_eval_worst` pattern.

- [ ] **Task 11. Implement `EvalOrchestrator::rerun`.**
  In `orch/crates/services/src/eval_orchestrator.rs`:
  1. POST to `/evals-be/api/evals/rerun` with `trigger = false` to deterministically invalidate.
  2. If the request scope is `Summary { summary_id }` and the caller asked for immediate execution, call `drive_one(summary_id, eval_version)` (the existing per-summary loop at `:361-442`) synchronously and return its `MetricResult`s alongside the `RerunResponse`.
  3. If scope is `AllSummaries` with immediate execution, call a new `drive_many(summary_ids, eval_version)` that reuses the `buffer_unordered(self.concurrency)` pattern from `run_cycle` (`:260-284`). Cap the fan-out at a configurable `eval_rerun_max_batch` (default 500) to avoid overwhelming brainatlas.
  4. Emit structured logs and metrics (`tracing::info!`) at start/end of each batch.
  **Rationale**: reuses the already-proven concurrency primitive rather than introducing a second worker pool.

- [ ] **Task 12. Add orch HTTP routes.**
  In `orch/crates/server/src/server.rs:88-89`, mount:
  - `POST /orch/api/evals/rerun` — body `RerunRequest`, response `RerunResponse`.
  - `POST /orch/api/evals/rerun/:summary_id` — convenience shortcut that fixes `scope = Summary { summary_id }` and reads `metric`, `eval_version`, `trigger` from query params.
  Add handlers analogous to `get_eval_status_handler` and `get_eval_worst_handler` (`:274-294`).
  **Rationale**: one canonical JSON endpoint plus a REST-shaped convenience URL for curl/UI use.

- [ ] **Task 13. Add configuration knobs for safety.**
  Add new orch_config rows (new migration under `orch/migrations/`): `eval_rerun_enabled` (default `'true'`), `eval_rerun_max_batch` (default `'500'`), `eval_rerun_require_version_bump` (default `'false'`). Mirror keys into `ConfigKey` at `orch/crates/domain/src/lib.rs:62-71`. The `require_version_bump` knob, when `true`, refuses bulk re-runs that would write under an unchanged `eval_version` — a production guardrail.
  **Rationale**: operators need the ability to disable destructive re-runs by config without a redeploy, matching the project-wide convention "every tuning knob must be config-table driven" (`plans/2026-04-19-evals-architecture-v1.md:144`).

- [ ] **Task 14. Respect `eval_orchestrator_enabled` semantics.**
  A bulk re-run with `trigger = true` must not start fan-out if `eval_orchestrator_enabled = false`; it should still invalidate and return 202 so the orch loop will pick up when re-enabled. Per-summary re-runs with `trigger = true` proceed regardless, because they run a single loop inline and don't depend on the poller.
  **Rationale**: keeps the master kill-switch honored.

### Phase 4 — Frontend: trigger UI

- [ ] **Task 15. Add a "Re-run" affordance in `atlas`.**
  Near `EvalScoresBar` (`atlas/src/components/detail/RegionDetail.tsx:488, 562-641`) add a small button/menu that POSTs to `/orch/api/evals/rerun/:summary_id`. Options: re-run all metrics, or pick a single metric from a dropdown of `EvalMetric::all()` strings. Show a spinner until the response returns and then re-fetch the scores via the existing `/orch/api/...` summary fetch.
  **Rationale**: lets neuroscientists compare scores before/after a prompt tweak without SSH-ing into the DB.

- [ ] **Task 16. Add a bulk trigger to `dev_stats.html`.**
  In `orch/crates/server/src/dev_stats.html:323-327` the "Evals" panel gains a "Rescore all (current version)" and "Rescore all (new version: …)" pair of buttons that POST `RerunRequest { scope: AllSummaries, selector: AllMetrics }`.
  **Rationale**: operator-facing UI for corpus-wide refresh, matching the file's existing read-only eval panel.

- [ ] **Task 17. Surface per-summary `eval_version` and `metric_version`.**
  Update `atlas/src/types/cortexmap.ts:13-27` (`SummaryEvalScores`) to carry `metric_version` per score, and adapt the tooltip in `EvalScoresBar` to display both `eval_version` and `metric_version` so users can see what scored rows correspond to what definition.
  **Rationale**: makes the newly introduced per-metric version observable end-to-end.

### Phase 5 — Testing

- [ ] **Task 18. Extend `cache_hit.rs` with invalidation tests.**
  In `evals-be/crates/app/tests/cache_hit.rs:1-493`:
  - Add a test that runs the full loop once, calls the new `EvalsApp::rerun` with per-summary scope, runs the loop again, and asserts the LLM was called a second time (no more `cached: true`).
  - Add a test that bumps `metric_version` for one metric (via a test-only extension trait on `EvalMetric`) and asserts only that metric is re-computed while the others return `cached: true`.
  - Add a test asserting `force: true` on `InitScoreRequest` has the same effect as `rerun` for a single summary.
  **Rationale**: locks the invalidation invariants that the immutable cache currently does not have coverage for.

- [ ] **Task 19. Add orch unit tests for `rerun` fan-out.**
  Using the existing orch test scaffolding (see `orch/crates/services/src/eval_orchestrator.rs` neighboring tests / stubs), add a test that `drive_many` respects `eval_rerun_max_batch` and `eval_orchestrator_concurrency`, and that disabling `eval_rerun_enabled` returns a 403-like error without issuing DELETEs.
  **Rationale**: guards the operator-facing knobs introduced in Task 13.

- [ ] **Task 20. Add an e2e smoke test.**
  In `tests/` (or `evals-be/crates/app/tests/`) add a test that spins up the Docker test infra (`docker-compose.test.yml`), seeds one `region_summary`, runs an eval once, POSTs `/orch/api/evals/rerun/:summary_id`, and verifies `eval_scores` got a new row with a later `created_at`.
  **Rationale**: proves the HTTP path, DB path, and orch fan-out integrate end-to-end.

### Phase 6 — Documentation & rollout

- [ ] **Task 21. Update the architecture plan.**
  Add a section to `plans/2026-04-19-evals-architecture-v1.md` describing the re-run primitive and the new `metric_version` column. Explicitly answer the open question at `:287` ("cache invalidation strategy") by citing the re-run endpoint and the per-metric version.
  **Rationale**: keeps the foundational plan as the source of truth; operators should be able to discover re-run semantics from one document.

- [ ] **Task 22. Operational runbook entries.**
  Add to the relevant per-service README (`evals-be/README.md` if present, else a top-level operators section in the main `README.md`) the canonical incantations:
  - Rescore one summary now: `curl -X POST .../orch/api/evals/rerun/:summary_id`.
  - Rescore all summaries under a new version: update `orch_config.eval_version`, then POST `/orch/api/evals/rerun` with `AllSummaries` + `AllMetrics`.
  - Rollback: keep old `eval_version` rows in `eval_scores` — they are not deleted.
  **Rationale**: minimizes the blast radius of re-runs by making the common operational flows obvious.

- [ ] **Task 23. Ship behind a feature flag and announce.**
  Initially set `eval_rerun_enabled = false` in production and `true` in dev. Verify a full corpus rescore on staging, then flip in production. Monitor `eval_orchestrator_concurrency` saturation and OpenRouter rate limits for one full cycle.
  **Rationale**: a corpus-wide re-run is the single most expensive operation evals-be can perform; a feature-flagged rollout protects against cost regressions.

---

## Verification Criteria

- A user can POST `/orch/api/evals/rerun/:summary_id` with `selector = Metric { metric: "rubric_relevance" }` and observe:
  - Exactly one new row in `eval_scores` with a `created_at` later than the prior row.
  - The other 14 metrics' cached rows untouched.
  - A fresh `MetricResult { cached: false }` for `rubric_relevance` in the response.
- A user can POST `/orch/api/evals/rerun` with `scope = AllSummaries` and observe the orch background loop picking up every active summary within one poll interval (`eval_orchestrator_poll_interval_secs`, default 60s).
- Bumping only `EvalMetric::RubricRelevance::version()` from `"v1"` to `"v2"` and re-running causes exactly one LLM call per summary (for `rubric_relevance`) with no re-compute of other metrics, proven by a test.
- Setting `eval_rerun_enabled = false` in `orch_config` causes `/orch/api/evals/rerun` to return an error without mutating the database.
- Historical `eval_scores` rows under the old `eval_version` remain queryable via `GET /evals-be/api/evals/scores/:summary_id` and appear in the UI as "previous version" data.
- Every existing test (`evals-be/crates/app/tests/cache_hit.rs`, plus domain unit tests) still passes unchanged except where it explicitly asserts the new `metric_version` column.
- The citation-metric WIP (`plans/2026-04-20-citation-correctness-evals-v1.md`) integrates without further API changes; a new `EvalMetric::Citation*` variant need only add itself to `EvalMetric::all()` and a `version()` entry.

---

## Potential Risks and Mitigations

1. **Cost blowout on bulk re-run.**
   A corpus-wide re-run that forces every metric across ~657 summaries is O(thousands) of LLM calls.
   *Mitigation*: `eval_rerun_max_batch` cap (Task 13); per-metric version lets operators narrow the blast radius to one metric at a time; `eval_rerun_enabled` feature flag (Task 13, Task 23); `eval_rerun_require_version_bump` guardrail forces a deliberate version change before bulk destructive re-runs.

2. **Race between invalidation and orch poll loop.**
   If `/rerun` deletes scores while orch is mid-way through `drive_one` for the same summary, the partial run could write back stale rows.
   *Mitigation*: The `eval_runs` row is upserted at the start of `init_score` (`evals-be/crates/app/src/app.rs:143-159`); `/rerun` deletes that row, so any in-flight `step_score` call targeting a now-deleted `eval_run_state` already errors out via the existing `pending_step_id` check (`app.rs:258-277`). Add a test for this specific interleaving in Task 19.

3. **Prompt-change drift without `metric_version` bump.**
   Developers may still forget to bump `EvalMetric::version()` after editing a prompt file in `brainatlas-be/crates/app/prompts/`.
   *Mitigation*: Introduce a CI check that hashes each prompt file and asserts a recorded `(prompt_hash, metric, metric_version)` table in `evals-be/crates/domain/` matches. Out of scope for v1 but flagged as follow-up in Task 21 (same footgun plan v1 calls out at `:287`).

4. **`run_id = Uuid::nil()` ambiguity on full-cache-hit.**
   The existing `init_score` returns `Uuid::nil()` when every metric is cached (`evals-be/crates/app/src/app.rs:223-228`). After a re-run with `force`, some metrics may still be cache-hits (e.g. structural metrics with unchanged summary text), so this zero-run can happen under `force` too.
   *Mitigation*: Document this explicitly in the `RerunResponse` shape; `invalidated` and `enqueued` vectors disambiguate "nothing to run" from "done trivially".

5. **Schema migration on a populated table.**
   Adding a NOT NULL column to `eval_scores` (potentially millions of rows over time) is locking on Postgres.
   *Mitigation*: The Diesel migration in Task 1 uses `ADD COLUMN ... DEFAULT 'v1' NOT NULL`, which in PG 11+ is metadata-only and does not rewrite the table. Verify this explicitly in staging.

6. **UI triggering re-runs from unauthenticated sessions.**
   Today the app has no auth layer (reads from `FIXME.md` context); exposing a destructive endpoint widens the attack surface.
   *Mitigation*: Gate `POST /orch/api/evals/rerun*` behind the same dev/ops protection used by `dev_stats.html`, or add a simple bearer-token check keyed off an `EVAL_RERUN_TOKEN` env var. Note this in the runbook (Task 22) and revisit when project-wide auth lands.

7. **`is_active` filter breaks legacy callers.**
   If some caller today scores on inactive summaries intentionally, Task 5 changes that.
   *Mitigation*: Keep `get_summary(summary_id)` unconditional (used by per-summary paths); only the new bulk-enumeration methods filter on `is_active`. Preserves every existing behavior.

---

## Alternative Approaches

1. **Version-bump only (no deletes).**
   Keep the cache strictly immutable and expose only "set a new `eval_version` and rescore everything".
   *Trade-offs*: Cheapest to implement (no new DELETE plumbing, no `metric_version`). But it is all-or-nothing corpus-wide, cannot target a single summary, and wastes compute when a single metric changed. Rejected as the primary path because it does not satisfy requirement (1) ("re-run one metric on one summary").

2. **Soft-delete via `superseded_by` column on `eval_scores`.**
   Instead of `DELETE`, mark rows as superseded and link forward.
   *Trade-offs*: Preserves full history inside one version; enables "time-travel" queries. But the UNIQUE index becomes harder to reason about (need `WHERE NOT superseded`), and all query paths must add the filter. Higher implementation cost and query complexity for a feature (history) that is not currently requested. Consider for v2.

3. **Per-run lineage table (`eval_run_history`).**
   Keep the current `eval_runs` upsert behavior but append every re-run to a new append-only table for audit.
   *Trade-offs*: Preserves audit trail without touching the hot cache path. But adds a new table and no one queries it today; same benefit is achievable by emitting structured log entries from the re-run endpoint. Defer unless an explicit audit requirement surfaces.

4. **Dedicated work queue (Redis / RabbitMQ) for re-runs.**
   Push re-run work onto a separate queue that a new worker pool consumes, rather than reusing orch's `buffer_unordered`.
   *Trade-offs*: Isolates re-run load from the steady-state poll loop. But introduces a second infra dependency (we already use Redis, so cost is small), a second code path to maintain, and duplicates the concurrency tuning knobs. Reusing the existing orch loop (Task 11) is simpler for v1; revisit if rescore load ever competes with real-time scoring.

5. **Client-driven loop.**
   Have the UI itself drive `/init`/`/step` calls (which it would need for first-time scoring anyway).
   *Trade-offs*: Removes orch from the rescore path. But `NextAction::CallLlm` currently targets brainatlas-be with full bodies constructed by evals-be; exposing that to the browser adds CORS, auth, and streaming complexity. Keep orch as the driver.
