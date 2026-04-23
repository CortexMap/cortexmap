# Max Test Coverage Plan — PR #69 (feat/continuous-pipeline-runner & evals)

## Objective

Drive the `feat/continuous-pipeline-runner` branch (PR #69) from the current **17.81% patch coverage / 1,869 missing lines** to the highest achievable coverage before merge. The PR introduces ~16,545 insertions across 145 files including an entirely new `evals-be` service, a continuous pipeline runner in `orch`, LLM cost tracking in `brainatlas-be`, and the evals orchestrator/state-machine wire protocol that spans two services.

Coverage wins must respect the existing patterns: trait-injected fakes with `Mutex<Vec<…>>` recording (no `mockall`, no `wiremock`), nightly `cargo llvm-cov` with `--test-threads=1`, and one CI matrix row per workspace.

---

## Assumptions

Made explicit so implementation agents do not second-guess:

1. **No new test frameworks.** Stay on `#[tokio::test]` + hand-rolled in-memory trait implementations, mirroring `evals-be/crates/app/tests/cache_hit.rs:38-206` and `brainatlas-be/crates/services/src/cost_accounting.rs:205-379`.
2. **`tracing-test` crate MAY be added** (dev-dep only) for asserting `cost_guardrail` alert levels and `llm.call` log field names — it is a tiny, well-scoped dep. If the maintainer prefers a hand-rolled `tracing::subscriber::with_default` subscriber, fall back to that.
3. **evals-be MUST be added to the CI matrix** and to the migration loop in `ci/tests/ci.rs:15-20` and `ci/tests/ci.rs:251-262`; otherwise new evals-be tests contribute zero to Codecov.
4. **Integration tests use unique random IDs** (`rand::random::<u16>() + 10000` pattern per `orch/crates/server/tests/integration_test.rs:245`) and clean up their own rows. Serial execution via `--test-threads=1` is already enforced.
5. **No refactor of production code** is in scope *except* the two narrow, low-risk seams explicitly listed in Task 3.1 and Task 3.8 (Clock trait for pricing TTL, shared URL-normalize helper). Any other refactor the tests demand is deferred.
6. **Coverage target by module**: ≥ 80% line coverage on the new `*.rs` files listed in the Codecov report, with realistic caveats for files that are pure wiring (e.g., `server/src/main.rs`).
7. **Running tests locally** requires `docker compose -f docker-compose.test.yml up -d` and `setup-test-data.sh`; this matches the existing developer workflow.

---

## Implementation Plan

Ordered strictly: cheapest-and-highest-ROI first, DB-backed next, wiring last. Each task is independently completable.

### Phase 0 — CI matrix & migration loop (unblocks everything else)

- [x] **Task 0.1. Add `evals-be` to the CI coverage matrix.** Extend the `include` array at `ci/tests/ci.rs:15-20` with `{"workspace": "evals-be", "lcov_name": "lcov-evals"}`. Update the `lcov --add-tracefile` command at `ci/tests/ci.rs:98-104` to include `lcov-evals.info`. Regenerate `.github/workflows/ci.yml` by running `cargo test -p ci`. Rationale: without this, every evals-be test written in later tasks is invisible to Codecov and the coverage number in the PR never moves.

- [x] **Task 0.2. Add evals-be migrations to the CI migration loop.** Update `run_migrations()` at `ci/tests/ci.rs:251-262` so it iterates `evals-be/migrations/*/up.sql` in addition to the existing three workspaces. Verify the new `eval_scores`, `eval_runs`, and `eval_run_state` tables exist after the step runs. Rationale: `evals-be/crates/infra/src/pg.rs` integration tests will fail with "relation does not exist" without this.

- [x] **Task 0.3. Ensure `autofix` workflow covers evals-be.** Mirror Task 0.1's matrix addition to the autofix workflow definition at `ci/tests/ci.rs:138-206`. Rationale: keep linting/formatting consistent so new tests don't fail autofix.

### Phase 1 — Pure unit tests (no DB, no network) — highest ROI per hour

- [x] **Task 1.1. State-machine direct coverage — `evals-be/crates/services/src/state_machine.rs`.** Add a `#[cfg(test)] mod advance_tests` alongside the existing tests at `state_machine.rs:1249-1386`. Import the `InMemoryDb` fake from `evals-be/crates/app/tests/cache_hit.rs:38-206` (copy or promote to a shared `test-support` helper module under `evals-be/crates/services/src/test_support.rs` gated on `#[cfg(test)]`). Write direct tests for every branch of:
    - `advance_claims` — empty claims short-circuit, malformed JSON, valid path
    - `advance_embed` — all-cached branch, partial-cache branch, empty chunks
    - `advance_judge` (groundedness) — each `GroundednessLabel` variant, missing verdict for a claim
    - `advance_rubric` — cached-rubric-but-fresh-groundedness path (the TODO at `state_machine.rs:622-644`)
    - `advance_citation_support` — truncation-at-budget (`state_machine.rs:1139-1164`), `find_next_support_step` iteration (`state_machine.rs:987-1003`), `enclosing_sentence_for_uuid` UTF-8 boundary handling (`state_machine.rs:1043-1068`)
    - Every response-type mismatch error path (`state_machine.rs:292-300`, `:339-347`, `:432-440`, `:539-547`, `:1094-1102`)
    - Round-trip of `RunState::AwaitingCitationSupport` through `serde_json::to_value` ↔ `from_value` with a non-trivial `cited_chunks: HashMap<Uuid, ChunkRow>` (guards the JSONB schema of `eval_run_state.state`).

- [x] **Task 1.2. Cache coverage — `evals-be/crates/services/src/cache.rs`.** Add unit tests covering `score_with_cache` (file is only 98 lines):
    - Cache hit path (`cached: true`, scorer not invoked)
    - Cache miss → scorer runs → insert succeeds
    - Concurrent-writer race: pre-seed the fake DB with a row that will cause `ON CONFLICT DO NOTHING` → re-SELECT path (`cache.rs:33-80`)
    - Scorer error propagates and no row is written

- [x] **Task 1.3. Cost guardrail alert levels — `orch/crates/services/src/cost_guardrail.rs`.** Create `orch/crates/services/src/cost_guardrail_tests.rs` (or inline `#[cfg(test)]`). Build a minimal in-memory `OrchDatabase` + `HttpClient` + `EnvInfra` fake that returns a staged `UsageAggregateWire`. Use `tracing::subscriber::with_default` or the `tracing-test` crate to capture output. Assert:
    - Both thresholds unset → short-circuit at `cost_guardrail.rs:114-119`, no log
    - `total_cost_usd >= daily_budget` → `error!` emitted with expected fields (`cost_guardrail.rs:147-157`)
    - `total_cost_usd >= warn_threshold && < daily_budget` → `warn!` emitted (`cost_guardrail.rs:158-168`)
    - Within bounds → `info!` emitted (`cost_guardrail.rs:169-176`)
    - HTTP failure → `None` returned, no panic (`cost_guardrail.rs:141-144`)
    - `poll_interval_secs()` defaults to 300 (`cost_guardrail.rs:61-66`)
    - `brainatlas_base_url()` env-var-first, config-fallback, `ConfigNotFound` error (`cost_guardrail.rs:68-95`)

- [x] **Task 1.4. Pipeline runner loop — `orch/crates/services/src/pipeline_runner.rs`.** Build an in-memory infra that records calls. Test:
    - `generate_queries_for_new_regions()` — empty input short-circuits (`pipeline_runner.rs:38+`)
    - Config-parsing edge cases for `QueryGenerationLimit`
    - Parallel per-region path via `stream::iter(...).buffer_unordered(N)` — assert concurrency cap
    - Per-region failure is swallowed (other regions still processed)
    - `skip_summarization` flag honors the phase skip
    - `AtomicBool` cancellation stops the loop after the current iteration
    - `backon::ExponentialBuilder.retry()` recovers from a transient failure
    - URL normalization helper (`pipeline_runner.rs:70-78`) — parametrized inputs (trailing `/`, scheme variants, IPv6)

- [x] **Task 1.5. Eval orchestrator — `orch/crates/services/src/eval_orchestrator.rs`.** Using an in-memory `HttpClient` that routes by URL/path:
    - Full happy path: init → one `CallLlm` → step → `Done` (mirrors `cache_hit.rs`)
    - Every `LlmEndpoint` variant at `eval_orchestrator.rs:79-96` routes to the right path
    - 5xx from brainatlas → retry succeeds on second attempt
    - 5xx sustained → fail-fast with classified error
    - Concurrent scoring of `N` unscored summaries via `stream::iter(...).buffered(K)` respects the cap
    - `GET /status` and `GET /worst` passthrough (no state machine involvement)

- [x] **Task 1.6. Completion watcher — `orch/crates/services/src/completion_watcher.rs`.** Unit tests for `CompletionWatcher::poll()` (`completion_watcher.rs:55-96`):
    - Transitions `collecting → ready`, `ready → processing`, `processing → completed`
    - Cache invalidation via `invalidate` / `invalidate_pattern` on status change (`completion_watcher.rs:83-87`)
    - No-op when status unchanged
    - URL normalization helper (`completion_watcher.rs:30-38`)

- [x] **Task 1.7. Region management — `orch/crates/services/src/region_management.rs`.** Unit tests:
    - `EvalsScoresWire` / `EvalsScoreEntryWire` serde round-trip with missing `judge_model` / `created_at` (backward compat) (`region_management.rs:21-40`)
    - Eval-scores fetch concurrency cap (`EVAL_SCORES_FETCH_CONCURRENCY = 16` at `region_management.rs:45`) — fake HTTP client that blocks until N in-flight
    - `get_summaries()` cache hit / miss path (`region_management.rs:72+`)
    - Wire-type numeric edge cases (float NaN, extreme values)

- [x] **Task 1.8. LLM service caller_tag semantics — `brainatlas-be/crates/services/src/llm_service.rs`.** Build a `MockInfra` (extend the one at `brainatlas-be/crates/services/src/cost_accounting.rs:219-283` or create a parallel one). Test:
    - For each of `summarize_with_tools` (`:37`), `generate_queries` (`:75`), `extract_claims` (`:133`), `judge_groundedness` (`:154`), `judge_rubric` (`:180`), and the citation-support judge: a caller-provided `caller_tag` is preserved end-to-end to `CostAccountant::finish`
    - Missing `caller_tag` falls back to the method default (`:57-65`, `:91-95`, `:145`, `:171`, `:192`)
    - `parse_json_loose` failure modes on malformed JSON (`:149`, `:175`, `:196`)
    - LLM error propagates without a usage row being recorded (or records a `cost_usd: NULL` row, whichever is the documented contract)

- [x] **Task 1.9. Cost accounting gap-fills — `brainatlas-be/crates/services/src/cost_accounting.rs`.** Extend the existing `#[cfg(test)]` block at `cost_accounting.rs:205-379`:
    - Embedding endpoint cost math (existing tests only cover `ChatCompletion`; see `outcome()` helper at `:296-303`)
    - Pricing cache **TTL expiry** — introduce a `Clock` trait seam (defaulting to `Instant::now`) at `cost_accounting.rs:100` so tests can advance time past the 300-second TTL (`cost_accounting.rs:21`). Seam is the only production-code refactor in Phase 1.
    - Full `UsageContext` round-trip: `correlation_id` / `region_id` / `summary_id` / `batch_id` propagate to the recorded row
    - Contract test with a `tracing::subscriber` that captures the `target: "llm.call"` event and asserts the field names (`prompt_tokens`, `completion_tokens`, `cost_usd`, …) are present and unrenamed (`cost_accounting.rs:127-142`) — downstream log pipeline depends on these exact names.

- [x] **Task 1.10. LLM infra gap-fills — `brainatlas-be/crates/infra/src/llm.rs`.** Existing tests at `:638-770+` cover `render_template`, `extract_first_string`, etc. Add:
    - Tool-calling loop multi-iteration usage aggregation (`infra.rs:123-132`)
    - `LlmCallOutcome` construction from empty / malformed usage blocks
    - Usage parsing when model omits optional fields

- [x] **Task 1.11. Serde round-trips — new wire types.** Parametrized tests for every public struct/enum in:
    - `orch/crates/domain/src/api_types.rs` (179 lines, new)
    - `orch/crates/domain/src/worker_types.rs` (46 lines, new)
    - `orch/crates/domain/src/lib.rs` (+24 lines)
    - `evals-be/crates/rpc-types/src/lib.rs` (176 lines) — critical: every `NextAction` and `LlmResponsePayload` variant, since these are the wire contract with orch
    - `brainatlas-be/crates/rpc-types/src/evals.rs` (113 lines, new)
  Assert: `serde_json::to_value(x) |> from_value == x` for a representative value of each variant.

- [x] **Task 1.12. Domain gap-fills — `brainatlas-be/crates/domain/src/{cost,evals,usage}.rs`.** Each file has `#[cfg(test)]` but coverage is partial. Extend:
    - `cost.rs`: `LlmPricing::compute_cost_usd` with embedding endpoint, zero tokens, missing embedding price, `BigDecimal` precision edge cases
    - `evals.rs`: every new enum/label round-trip + `Display`/`FromStr` if present
    - `usage.rs`: `UsageAggregate` arithmetic with mixed-endpoint sums

- [x] **Task 1.13. Evals-be domain & config — `evals-be/crates/domain/src/{config,evals,hash}.rs`.** Tests already exist (`hash.rs:4`, `evals.rs:5`, `config.rs:2`). Fill gaps:
    - `hash.rs`: hash stability across minor whitespace, unicode-normalization edge cases
    - `config.rs`: env-variable parsing, default values, invalid inputs
    - `evals.rs`: every metric-name / `GroundednessLabel` / `RubricCriterion` round-trip

- [x] **Task 1.14. Citations & structural metrics gap-fills — `evals-be/crates/services/src/citations.rs` and `structural.rs`.** Both files already have comprehensive tests (`citations.rs:426-718`, `structural.rs:89-192`). Verify full-coverage by inspection; add only demonstrated gaps. Likely missing:
    - `citations.rs`: malformed fenced-code-block edge case mid-sentence
    - `citations.rs`: UUID case-sensitivity across the presence/validity/scope/support four metrics (the formulas must agree)

### Phase 2 — Integration tests (real Postgres + Redis)

- [x] **Task 2.1. Evals-be DB — `evals-be/crates/infra/src/pg.rs`.** Create `evals-be/crates/infra/tests/pg_integration.rs` following the pattern at `fetcher-be/crates/std-infra/tests/task_queue_tests.rs:1-108`. For every method on `EvalsDatabase`:
    - `lookup_score_by_hash`, `insert_score` — unique-constraint round-trip
    - `get_summary`, `get_summary_with_chunks`
    - `save_run_state`, `load_run_state` — JSONB round-trip for every `RunState` variant (guards Task 1.1 on the real SQL)
    - `list_worst_offenders`, `aggregate` with each filter permutation
    - `eval_runs` lifecycle: insert → update status → query by `eval_version`

- [x] **Task 2.2. Orch DB — `orch/crates/infra/src/pg.rs`.** Create or extend `orch/crates/server/tests/pg_integration.rs`. Critical regression: the `is_active` → `summary IS NOT NULL` migration at `orch/crates/services/src/infra.rs:374-388`. Seed two rows: (a) `is_active=false AND summary IS NOT NULL`, (b) `is_active=true AND summary IS NULL`. Verify (a) now appears in `get_latest_active_summary_age` / `get_summary_freshness_counts` and (b) does not.

- [x] **Task 2.3. Redis cache — `orch/crates/infra/src/redis.rs`.** Create `orch/crates/infra/tests/redis_integration.rs`. Uses the `redis-test` service from `docker-compose.test.yml` (already wired in `ci/tests/ci.rs:220-222`). Test:
    - `cache_set` → `cache_get` round-trip (JSON + bytes)
    - TTL expiry (seeded ttl=1s, sleep, assert None)
    - `cache_del_pattern` with glob matches multiple keys
    - `cache_stats` when Redis reachable, gracefully degraded when connection fails
    - Every trait method from `orch/crates/services/src/infra.rs:395-417`

- [x] **Task 2.4. Brainatlas LLM usage repo — `brainatlas-be/crates/infra/src/llm_usage.rs`.** Create integration tests against the `llm_pricing` and `llm_call_usage` tables:
    - `latest_for_model` ordering by `effective_from DESC` with multiple rows
    - `record` insert round-trip with `BigDecimal::from_f64` precision at the edges (very small, very large, zero)
    - `aggregate` with every filter permutation: `since`, `until`, `model`, `correlation_id`, `correlation_id_prefix`, `region_id`, `summary_id`, `batch_id`, `caller_tag`
    - **Critical: `correlation_id_prefix` LIKE-escape at `llm_usage.rs:161`** — seed rows whose IDs contain `%`, `_`, `\` and verify exact prefix matching.
    - `null_pricing_insert` path (model not in pricing table)

- [x] **Task 2.5. Task queue duplicate-ID regression — `fetcher-be/crates/std-infra/src/task_queue.rs:53-108`.** The existing test `test_duplicate_task_handling` at `fetcher-be/crates/std-infra/tests/task_queue_tests.rs:81-108` must assert the NEW contract: `.on_conflict(fetch_tasks::pmc_id).do_nothing()` returns the **existing** row, not a new insert, not an error. If the current assertion only checks "no error", strengthen it to compare the returned row's `id` / `created_at` against the pre-existing one.

- [x] **Task 2.6. Migration round-trip tests.** One test per new migration, placed under each workspace's existing integration test harness:
    - `evals-be/migrations/2026-04-19-000001-create_eval_scores/{up,down}.sql`
    - `evals-be/migrations/2026-04-19-000002-add_eval_run_state/{up,down}.sql`
    - `brainatlas-be/migrations/2026-04-20-000001-add_llm_pricing/{up,down}.sql` — additionally verify the three seed rows at `up.sql:25-30` are present
    - `brainatlas-be/migrations/2026-04-20-000002-add_llm_call_usage/{up,down}.sql` — verify all six indexes exist
  Each test: `run up → assert expected tables/indexes → run down → assert objects gone → re-run up for idempotency`.

### Phase 3 — HTTP handler / end-to-end tests

- [ ] **Task 3.1. (Optional refactor) Extract duplicate URL-normalize helper.** The same logic appears at `orch/crates/services/src/pipeline_runner.rs:70-78`, `cost_guardrail.rs:69-76`, `completion_watcher.rs:30-38`. Move to a shared `orch/crates/services/src/url_util.rs` with a single set of parametrized tests. Reduces test surface from 3× to 1×. **Only if maintainer approves the refactor** — otherwise leave duplication and test each site once.

- [x] **Task 3.2. Orch axum handlers — `orch/crates/api/src/api.rs` and `orch_api.rs`.** Create `orch/crates/api/tests/handler_test.rs`. Use `tower::ServiceExt::oneshot` against the real `Router`. Cover:
    - Every new route added in this PR (manual pipeline trigger, per-phase opt-in, redis stats, dev-dashboard endpoints)
    - 400 / 404 / 500 paths
    - Request body validation
    - Response shape matches `orch/crates/domain/src/api_types.rs`

- [x] **Task 3.3. Brainatlas axum handlers — `brainatlas-be/crates/server/src/server.rs` (+158).** Same pattern. Cover:
    - `/api/llm/usage` aggregation endpoint (every query param filter)
    - Evals-orchestration manual trigger endpoint
    - Cost reporting endpoints
    - Unauthorized / malformed request paths

- [x] **Task 3.4. Evals-be axum handlers — `evals-be/crates/api/src/api.rs` (121 lines).** Same pattern. Cover every route: `/init_score`, `/step`, `/status`, `/worst`, health.

- [x] **Task 3.5. Orch ↔ evals-be contract test (wire protocol).** Place under `orch/crates/services/tests/eval_loop_contract.rs`. Instantiate `orch::EvalOrchestrator` and `evals::EvalsApp` in the same process with an in-memory HTTP router that shuttles requests between them (route by path on the fake `HttpClient`). Drive a full `init_score` → `step`* → `Done` flow. Asserts the two halves stay in lockstep as the wire protocol evolves — highest-value single test in the PR.

- [x] **Task 3.6. Extend orch server integration test — `orch/crates/server/tests/integration_test.rs`.** Follow the existing raw-SQL pattern (`:60-64, :245`) plus random IDs. Add tests for:
    - Manual pipeline trigger endpoint with each per-phase opt-in combo (the `skip_summarization` flag, parallel query-gen toggle)
    - Redis stats endpoint happy path + Redis-down degraded path
    - Dev-dashboard panel data endpoints

- [x] **Task 3.7. Extend brainatlas server integration test — `brainatlas-be/crates/server/tests/integration_test.rs`.** Add tests for new LLM-usage and evals endpoints.

- [ ] **Task 3.8. (Optional seam) `Clock` trait for pricing cache.** Only needed for Task 1.9's TTL test. Add a tiny `trait Clock { fn now(&self) -> Instant; }` defaulting to `SystemClock`, inject into `CostAccountant`. Keep change < 30 lines.

### Phase 4 — Coverage audit & gap close

- [x] **Task 4.1. Run `cargo llvm-cov --html` locally per workspace.** Generate per-file HTML reports and identify any file still below 80% line coverage.

- [x] **Task 4.2. Close remaining gaps.** For each file below threshold, add targeted tests using the pattern established in the relevant phase above. Expected remaining offenders likely to be:
    - `orch/crates/server/src/server.rs` (+86 lines of wiring) — wiring is inherently hard to unit-test; ensure the axum-handler tests in Task 3.2 cover it
    - `orch/crates/app/src/app.rs` (+507 lines) — large file; may need a dedicated task

- [x] **Task 4.3. Verify Codecov patch-coverage diff.** After PR push, confirm Codecov bot shows ≥ 80% patch coverage (vs. current 17.81%). If specific files remain stubborn, add an explicit justification comment (e.g., main-binary entry points).

- [x] **Task 4.4. Ensure `--test-threads=1` still holds.** Confirm no test relies on parallel execution. Any parallel-dependent test will flake against the shared Postgres.

- [~] **Task 4.5. Document the test-support fake in a rustdoc comment.** If `InMemoryDb` is promoted to a shared module in Task 1.1, add a rustdoc comment describing the contract and its relation to the real `EvalsDatabase` impl at `evals-be/crates/infra/src/pg.rs`. (Rustdoc only — not a separate markdown file.)

---

## Verification Criteria

- **Codecov patch coverage for PR #69 ≥ 80%** (currently 17.81% per PR comment).
- **Per-file line coverage ≥ 80%** for every file listed in the Codecov "Files with missing lines" table, excluding:
  - `evals-be/crates/server/src/main.rs` (binary entry point)
  - `orch/crates/server/src/dev_stats.html` (static asset, not Rust)
- **Zero new test failures in CI.** `cargo +nightly llvm-cov --all-features --workspace --lcov --output-path … -- --test-threads=1` passes for all four workspaces including the newly added `evals-be` row.
- **All four new migrations have round-trip tests** that exercise `up.sql → down.sql → up.sql` in sequence.
- **The evals-be/orch wire-protocol contract test** (Task 3.5) passes and exercises every `NextAction` and `LlmEndpoint` variant.
- **The `is_active → summary IS NOT NULL` regression is explicitly covered** — seed rows with the problematic states are asserted against the new filter (Task 2.2).
- **The LIKE-escape branch at `llm_usage.rs:161`** has a test with a `correlation_id` containing each of `%`, `_`, `\` (Task 2.4).
- **`tracing::info!(target: "llm.call", …)` field-name contract test** exists (Task 1.9) guarding the downstream log pipeline.
- **CI workflow regenerates cleanly** — `cargo test -p ci` followed by `git diff .github/workflows/` shows only intended changes.

---

## Potential Risks and Mitigations

1. **Coverage tool disagrees with expectations.** `cargo llvm-cov` counts branches differently from `tarpaulin`; a file reported 60% locally may show 75% on Codecov.
   Mitigation: use Codecov's PR comment as the source of truth, not local `--summary-only`.

2. **Integration tests flake under `--test-threads=1`** due to shared Postgres state across tests in the same workspace.
   Mitigation: strictly follow the `rand::random::<u16>() + 10000` ID pattern per `orch/crates/server/tests/integration_test.rs:245`; every test cleans up its own rows; never rely on DB ordering.

3. **`evals-be` not in CI matrix means new tests contribute zero coverage.** Already called out as Task 0.1.
   Mitigation: ship Task 0.1 first, in its own commit, and verify on GitHub Actions before proceeding.

4. **The `Clock` seam for Task 1.9 is the only production refactor.** If maintainer rejects it, TTL expiry remains untested.
   Mitigation: skip Task 1.9 TTL sub-item; accept ~5% coverage shortfall on `cost_accounting.rs`; file a follow-up issue.

5. **The evals-be ↔ orch contract test (Task 3.5) spans two workspaces.** `cargo llvm-cov` runs per workspace, so cross-workspace coverage attribution is fuzzy.
   Mitigation: place the test file physically under `orch/crates/services/tests/` and add an explicit dev-dep on `evals-be` crates; coverage attributes to `orch` workspace.

6. **`tracing-test` dev-dep may be rejected by maintainer.** Backup: hand-rolled `tracing::subscriber::with_default` + a `Vec<String>`-backed layer (~30 lines).
   Mitigation: prepare both variants in the PR; let the maintainer pick.

7. **Hand-rolled fakes per trait are verbose.** Risk of test maintenance burden.
   Mitigation: promote the `InMemoryDb` fake to a `test-support` module in each workspace (`#[cfg(test)]` gated) so new tests import it instead of re-implementing.

8. **The `save_run_state` JSONB round-trip may reveal schema drift between `state_machine.rs` and `pg.rs`.** The `cited_chunks: HashMap<Uuid, ChunkRow>` field is large and could hit Postgres `jsonb` practical limits.
   Mitigation: Task 2.1 explicitly asserts round-trip equality for every `RunState` variant; Task 1.1 adds a size budget check.

9. **Refactor fatigue — the plan touches ~40 files.**
   Mitigation: split into multiple commits along Phase boundaries; each phase should be independently mergeable.

10. **Hidden global state in `app.rs` or `server.rs`.** If either file depends on a `once_cell::Lazy` or `std::sync::OnceLock`, tests may interfere with each other.
    Mitigation: pre-read `orch/crates/app/src/app.rs` and `brainatlas-be/crates/app/src/app.rs` before writing handler tests; flag any `Lazy` / `OnceLock` for a separate refactor ticket.

---

## Alternative Approaches

1. **"Fire-and-forget: only integration tests via docker-compose."** Spin up all services end-to-end in a single `tests/e2e/` harness, drive via HTTP. *Trade-off*: fast to write, but slow to run, high flakiness, poor line-level attribution on Codecov. Reject for the main effort; use only for Task 3.5-style contract tests.

2. **"Introduce `mockall`."** Generate mocks for every trait automatically. *Trade-off*: reduces boilerplate, but breaks the existing convention used by `cost_accounting.rs:219-283` and `cache_hit.rs:38-206`. Would require converting existing tests to the new style for consistency — out of scope for this PR. Defer to a standalone refactor PR.

3. **"Ship new tests in a follow-up PR after merge."** Merge PR #69 at 17.81% coverage, then file a coverage-only PR. *Trade-off*: faster to ship the feature, but defers risk; bugs in untested paths (e.g., the LIKE-escape branch, state-machine response-mismatch error paths, `is_active` regression) will reach production first. Strongly prefer in-PR testing.

4. **"Focus only on the four highest-line-count files (eval_orchestrator, app.rs, region_management, server.rs)."** Would move the needle from 17.81% → ~55% with ~5 days of effort instead of the full plan's ~10-14 days. *Trade-off*: ignores high-risk small files (`cost_guardrail.rs`, `llm_usage.rs` escape logic). Reasonable compromise if timeline-constrained; use Phases 0–1 + Tasks 2.2, 2.4, 2.5 as the "minimum viable" subset.

5. **"Property-based tests via `proptest` for serde round-trips."** Replaces Task 1.11's explicit parametrized tests with `proptest`-generated inputs. *Trade-off*: catches more edge cases, adds a dep, increases CI time by ~15%. Nice-to-have; defer unless a serde bug is actually suspected.
