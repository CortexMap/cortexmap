# Evals Architecture — v1

## Status Snapshot

Greenfield. 657 summaries currently exist in `region_summary` (611 non-empty, avg 5021 chars, avg 140 source chunks per summary). The generation pipeline writes to `region_summary.summary` and `brain_region_embeddings` via brainatlas. No quality signal currently exists — failures, hallucinations, and degenerate outputs are invisible.

This plan scaffolds a three-service evaluation system (orch -> evals-be -> brainatlas) that produces per-metric scores for every summary, persists them to a new `eval_scores` table, and surfaces them on `/dev/stats`. Rollout is staged so structural metrics ship first (free, no LLM dependency), LLM-judge metrics second.

## Objective

For every active summary in `region_summary`, produce a normalized `score` (0.0–1.0) across three categories of metrics:

1. **Structural (deterministic)** — section completeness, length bounds, acronym mention, placeholder detection. Pure Rust, no LLM.
2. **Groundedness (LLM + retrieval)** — extract atomic claims from the summary, retrieve supporting chunks from `brain_region_embeddings`, ask a judge LLM whether each claim is grounded. Produces `claim_groundedness` and `hallucination_rate`.
3. **Quality rubric (LLM)** — score relevance, coherence, specificity, clinical utility, terminology against a fixed rubric. Five sub-scores.

All LLM work stays in brainatlas (consistent with the existing architecture). Orch schedules and polls. A new `evals-be` crate owns the eval pipeline logic, the `eval_scores` DB, and the public API.

## Architectural Constraints

- **Orch** schedules and polls; never touches LLM providers directly.
- **Brainatlas** owns all LLM/embedding calls; stateless endpoints only for evals (no eval-specific DB writes).
- **Evals-be** is the new service; it owns the `eval_scores` table, the eval pipeline logic, retrieval against `brain_region_embeddings`, and the public score API.
- Eval work must not block the main summary pipeline — failures in evals-be or the judge model must not propagate back into `region_processing_batches`.
- **Scores are cached by summary content hash.** Every score row is keyed by `(summary_hash, metric, eval_version)`. If the same summary text is evaluated twice — whether via a regeneration that produced identical output, a manual re-score, or a different `summary_id` with matching text — the cached score is returned immediately with no LLM call and no structural recomputation. This is a first-class correctness and cost requirement, not an optimization.

## Service Topology

```
orch  (Phase 4: eval orchestrator)
  |  POST /evals-be/api/evals/score {summary_id, eval_version}
  v
evals-be  (new crate workspace)
  |  - Loads summary + region + chunks from DB (direct access)
  |  - Runs structural metrics locally
  |  - POST /brainatlas-be/api/llm/extract-claims
  |  - POST /brainatlas-be/api/llm/embed  (per claim, for retrieval)
  |  - pgvector similarity query against brain_region_embeddings
  |  - POST /brainatlas-be/api/llm/judge-groundedness  (per claim)
  |  - POST /brainatlas-be/api/llm/judge-rubric       (once per summary)
  |  - Writes eval_scores rows
  v
brainatlas-be  (reused LlmService + EmbeddingService + new endpoints)
```

## What's Already in Place (do not re-touch)

- `region_summary` table with 657 rows (`region_summary_pkey`, `idx_region_summary_active`)
- `brain_region_embeddings` table with IVFFLAT index on `embedding vector_cosine_ops`
- `LlmService::summarize_with_tools` at `brainatlas-be/crates/services/src/services.rs:60`
- `EmbeddingService::generate_embedding` at `brainatlas-be/crates/services/src/services.rs:89`
- Prompt templating pattern via `include_str!("../prompts/*.md")` in `brainatlas-be/crates/app/src/app.rs:16-17`
- Structured output via `schemars::schema_for!(T)` pattern in `brainatlas-be/crates/app/src/app.rs:225-227`
- Background task pattern with `buffer_unordered(concurrency)` in `orch/crates/services/src/completion_watcher.rs:181-194`
- Dev dashboard auto-polling at `orch/crates/server/src/dev_stats.html`
- `/orch/dev/api/system-stats` aggregate endpoint pattern

## Implementation Plan

### Step 1 — Brainatlas LLM Judge Endpoints (Stateless)

Add three new stateless LLM endpoints to brainatlas. No DB writes, no eval-specific state — pure LLM wrappers that any caller can use.

- [x] Task 1.1. Create prompt template `brainatlas-be/crates/app/prompts/extract_claims_system.md`. The prompt instructs the model to split a summary into atomic factual claims, each tagged with its section heading, returning structured JSON matching a `ClaimsResponse { claims: Vec<Claim> }` schema where `Claim = { id: u32, section: String, text: String }`. Rationale: deterministic structured output removes parsing fragility.

- [x] Task 1.2. Create prompt template `brainatlas-be/crates/app/prompts/judge_groundedness_system.md`. The prompt gives the model a single claim plus N candidate evidence chunks (numbered) and asks for a verdict (`supported`, `partial`, `contradicted`, `unsupported`), a confidence score 0.0–1.0, a list of supporting chunk indices, and a one-sentence rationale. Rationale: per-claim judgment is more reliable than whole-summary judgment and produces actionable drill-down data.

- [x] Task 1.3. Create prompt template `brainatlas-be/crates/app/prompts/judge_rubric_system.md`. The prompt gives the model the full summary and asks it to score five criteria on 1–5: relevance, coherence, specificity, clinical_utility, terminology. Each criterion requires a score and one-sentence rationale. Rationale: fixed rubric + structured output lets scores aggregate across runs.

- [x] Task 1.4. Add `domain::ClaimsResponse`, `domain::GroundednessVerdict`, `domain::RubricScores` types in `brainatlas-be/crates/domain/src/` with `JsonSchema` + `Serialize`/`Deserialize` derives. Reuse the `schemars::schema_for!` pattern. Rationale: types stay in domain so all layers use the same wire format.

- [x] Task 1.5. Add three methods to `brainatlas-be/crates/services/src/services.rs`'s `LlmService` trait: `extract_claims(summary_text, region_name, chat_model) -> ClaimsResponse`, `judge_groundedness(claim, evidence_chunks, chat_model) -> GroundednessVerdict`, `judge_rubric(summary_text, region_name, chat_model) -> RubricScores`. All implemented in `llm_service.rs` using `summarize_with_tools` with empty tools + structured output. Rationale: reuses existing LLM infrastructure (retry, timeout, model override).

- [x] Task 1.6. Add three HTTP handlers in `brainatlas-be/crates/server/src/` matching the pattern of the existing `/api/process` handler: `POST /api/llm/extract-claims`, `POST /api/llm/judge-groundedness`, `POST /api/llm/judge-rubric`. Request/response types live in `rpc-types`. Rationale: parallels the existing process-region endpoint so nothing in the infra layer changes.

- [x] Task 1.7. Confirm `/api/llm/embed` exists or add it. If `EmbeddingService::generate_embedding` has no HTTP counterpart, add `POST /api/llm/embed {text, embedding_model} -> {embedding: Vec<f32>}` in the server layer. Rationale: evals-be needs to embed claim text to retrieve supporting chunks; reusing the same model as summarization ensures comparable similarity scores.

- [x] Task 1.8. `cargo check -p brainatlas-be` and smoke-test each endpoint against a sample summary with `curl`. Rationale: LLM layer must be stable before evals-be depends on it.

### Step 2 — Evals Database Schema

- [x] Task 2.1. Create a new migrations directory for evals (`evals-be/migrations/` — evals-be owns its schema, separate diesel config). Scaffold `evals-be/diesel.toml` pointing to a new `schema.rs`. Rationale: keeps eval schema independent of orch's schema; follows the same pattern orch/fetcher-be already use.

- [x] Task 2.2. Create migration `evals-be/migrations/<timestamp>-create_eval_scores/up.sql`:
  ```sql
  CREATE TABLE eval_scores (
      id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
      summary_id UUID NOT NULL REFERENCES region_summary(id) ON DELETE CASCADE,
      summary_hash VARCHAR(64) NOT NULL,  -- SHA-256 hex digest of region_summary.summary at score time
      metric VARCHAR(64) NOT NULL,
      score REAL NOT NULL,
      judge_model VARCHAR(128),
      details JSONB,
      eval_version VARCHAR(16) NOT NULL,
      created_at TIMESTAMP DEFAULT NOW()
  );
  -- Cache key: identical summary text yields identical scores per (metric, eval_version).
  CREATE UNIQUE INDEX ix_eval_scores_cache ON eval_scores(summary_hash, metric, eval_version);
  CREATE INDEX ix_eval_scores_summary ON eval_scores(summary_id);
  CREATE INDEX ix_eval_scores_metric_score ON eval_scores(metric, score);
  ```
  Matching `down.sql` drops the indexes and table. Rationale: the unique index on `(summary_hash, metric, eval_version)` **is** the cache — any re-score of identical text short-circuits to the existing row via `SELECT ... WHERE summary_hash = $1 AND metric = $2 AND eval_version = $3`. `summary_id` is retained as a FK for join-back to `region_summary` and for the non-unique `ix_eval_scores_summary` index used by `GET /evals/scores/:summary_id`, but it is **not** part of the cache key. Bump `eval_version` to force re-evaluation of every summary regardless of hash.

- [x] Task 2.3. Create a companion `eval_runs` table in the same migration to track per-summary run status (so orch can tell "queued", "running", "complete", "failed" without parsing score rows):
  ```sql
  CREATE TABLE eval_runs (
      id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
      summary_id UUID NOT NULL REFERENCES region_summary(id) ON DELETE CASCADE,
      eval_version VARCHAR(16) NOT NULL,
      status VARCHAR(16) NOT NULL,        -- 'queued', 'running', 'complete', 'failed'
      error_message TEXT,
      started_at TIMESTAMP,
      completed_at TIMESTAMP,
      created_at TIMESTAMP DEFAULT NOW()
  );
  CREATE UNIQUE INDEX ix_eval_runs_unique ON eval_runs(summary_id, eval_version);
  CREATE INDEX ix_eval_runs_status ON eval_runs(status);
  ```
  Rationale: orch needs a lightweight "is this summary evaluated?" lookup without scanning all metric rows.

### Step 3 — Evals-be Crate Workspace Scaffold

Follow the existing orch/brainatlas-be crate layout. This is workspace plumbing only — the real work is in later steps.

- [x] Task 3.1. Create `evals-be/Cargo.toml` as a Rust workspace with crates: `domain`, `infra`, `services`, `app`, `api`, `server`. Copy `orch/Cargo.toml` as the template, update crate names and paths. Rationale: mirrors the existing service structure so operators and developers see the same pattern.

- [x] Task 3.2. Create `evals-be/crates/domain/src/lib.rs` with the core types:
  - `EvalScore { id: Uuid, summary_id: Uuid, summary_hash: String, metric: String, score: f32, judge_model: Option<String>, details: Option<serde_json::Value>, eval_version: String, created_at: NaiveDateTime }`
  - `EvalRun { id: Uuid, summary_id: Uuid, eval_version: String, status: EvalRunStatus, ... }`
  - `EvalRunStatus` enum with strum derives (`queued`, `running`, `complete`, `failed`)
  - `EvalMetric` enum with all known metric keys (`SectionCompleteness`, `ClaimGroundedness`, `RubricRelevance`, etc.) with `strum::IntoStaticStr` for DB column serialization
  - Re-export `compute_hash` from `brainatlas-be/crates/domain/src/hash.rs` (SHA-256, 64-char hex) or duplicate it here so evals-be does not cross-crate depend on brainatlas-be domain. Rationale: the hash function is the contract between writer and reader of the cache; both sides must use byte-identical input (`&region_summary.summary`) and algorithm.
  Rationale: strongly-typed metric names avoid string typos across the codebase and provide a single source of truth for metric enumeration.

- [x] Task 3.3. Create `evals-be/crates/domain/src/config.rs` with `ConfigKey` enum (following the pattern at `orch/crates/domain/src/lib.rs:14-62`):
  - `EvalVersion` — default `"v1.0"`
  - `EvalConcurrency` — default `5`
  - `EvalJudgeChatModel` — default `"openai/gpt-4o-mini"` for cheap metrics, override per-metric via env
  - `EvalRubricChatModel` — default `"openai/gpt-4o"` (stronger model for rubric)
  - `EvalEmbeddingModel` — default `"text-embedding-3-small"` (must match summary embedding model)
  - `EvalTopKChunks` — default `5` (chunks retrieved per claim)
  - `EvalSimilarityThreshold` — default `0.6` (reject retrieved chunks below this)
  - `BrainatlasBaseUrl` — reused from orch config pattern
  Rationale: every tuning knob must be config-table driven, not env-only, so operators can adjust without restarts (matches the precedent set in orch).

- [x] Task 3.4. Create `evals-be/crates/infra/src/schema.rs` generated by `diesel print-schema` after running migrations. Add `pg.rs` with `EvalsDatabase` trait implementation. Required methods for the cache path:
  - `lookup_score_by_hash(summary_hash: &str, metric: &str, eval_version: &str) -> Option<EvalScore>` — the cache read; indexed lookup on `ix_eval_scores_cache`.
  - `insert_score(new: NewEvalScore) -> EvalScore` — the cache write; use `INSERT ... ON CONFLICT (summary_hash, metric, eval_version) DO NOTHING RETURNING *` to handle the race where two concurrent runs score the same hash (return the existing row on conflict via a follow-up select).
  - `write_run`, `query_unscored_summaries`, etc. for the orchestrator path.
  Model it on `orch/crates/infra/src/pg.rs`. Rationale: direct DB access for evals_scores; the main app data (region_summary, brain_region_embeddings) is read-only via raw sql_query or a shared read-only schema module. The cache lookup must be a single indexed query — no N+1 patterns.

- [x] Task 3.5. Create stub `app.rs`, `services.rs`, `api.rs`, `server.rs` following the orch/brainatlas-be patterns. Empty handlers are acceptable at this stage; Step 4 fills them in. Rationale: gets `cargo check -p evals-be` green so later steps can iterate quickly.

- [x] Task 3.6. Create `evals-be/Dockerfile` by copying `orch/Dockerfile` and adjusting the crate name. Add evals-be service entry in `docker-compose.app.yml` with its DB URL and brainatlas URL envs. Rationale: deployment is at least as important as the code path; this reserves the container slot.

### Step 4 — Structural Metrics (No LLM, Ships First)

Implement the four cheapest metrics first. These validate the full DB write + API path (including the cache layer) without any LLM dependency, so you can see end-to-end score rows within an hour of landing.

- [x] Task 4.0. Add `evals-be/crates/app/src/cache.rs` with a single `score_with_cache(summary_hash, metric, eval_version, compute: impl FnOnce() -> Future<Score>) -> EvalScore` helper:
  1. `SELECT` from `eval_scores WHERE summary_hash = $1 AND metric = $2 AND eval_version = $3`. On hit, return the row and emit a `metric=eval_cache_hit` log line tagged with the metric name — **no compute runs**.
  2. On miss, invoke `compute()`, then `INSERT ... ON CONFLICT DO NOTHING` and re-select to resolve concurrent writers to the same row.
  3. The helper is the **only** path that writes to `eval_scores`. Structural metrics, groundedness, and rubric all go through it so the cache is impossible to bypass.
  Rationale: centralizing the read-through-cache pattern (analogous to `orch/crates/services/src/cache_keys.rs:89-124`) makes the caching behavior a property of the framework, not of individual metric implementations. A new metric added later cannot accidentally skip the cache.

- [x] Task 4.1. Add `evals-be/crates/services/src/structural.rs` with one pure function per metric:
  - `section_completeness(summary: &str) -> f32` — checks for the 6 required section headings (`## Overview`, `## Anatomy & Connectivity`, `## Functions`, `## Associated Disorders`, `## Symptoms of Damage or Dysfunction`, `## Research Highlights`). Score = found / 6.
  - `length_in_range(summary: &str) -> f32` — returns 1.0 if length in `[1500, 10000]`, else linear falloff to 0.0 at 0 or 20000.
  - `acronym_mention(summary: &str, acronym: Option<&str>) -> f32` — 1.0 if acronym present at least once, 0.0 otherwise. Returns 1.0 if acronym is None (some regions have no acronym).
  - `no_placeholder_text(summary: &str) -> f32` — 0.0 if any of `"TBD"`, `"TODO"`, `"Lorem ipsum"`, `"[placeholder]"`, `"(to be filled)"` appears, else 1.0.
  Rationale: all four are pure functions, no I/O, trivially unit-testable; they form the deterministic backbone of the eval score.

- [x] Task 4.2. Add unit tests for each structural metric in `evals-be/crates/services/src/structural.rs` covering edge cases (empty summary, missing sections, borderline lengths). Rationale: metrics are specifications — tests lock the scoring formula so regressions are detectable.

- [x] Task 4.3. Add `evals-be/crates/app/src/run_eval.rs` with a `run_structural_metrics(summary_id)` function that:
  1. Loads `region_summary.summary` and `region_summary.acronym` from DB.
  2. Computes `summary_hash = compute_hash(&summary_text)` **once** and passes it into every metric call. All metric rows produced by this invocation share the same hash.
  3. For each structural metric, calls `cache::score_with_cache(summary_hash, metric_name, eval_version, || async { structural::<fn>(&summary) })`. Each call either returns a cached row instantly or computes-and-inserts.
  4. Writes/updates the `eval_runs` row to `complete` once all structural metrics have resolved (cached or freshly computed are both "resolved").
  Rationale: a single entry point makes re-runs and backfills trivial, and guarantees every score write passes through the cache layer. Re-scoring an identical summary becomes a handful of indexed SELECTs with zero compute.

- [x] Task 4.4. Add `POST /evals-be/api/evals/score {summary_id, eval_version?}` handler in `evals-be/crates/server/src/`. For now, calls only `run_structural_metrics`. LLM metrics wired in Step 5. The response includes a per-metric `cached: bool` field so callers can observe cache effectiveness. Rationale: stable HTTP contract from day one — orch can integrate against this endpoint immediately, and the `cached` flag lets the dashboard show cache hit-rate without a separate metrics pipe.

- [x] Task 4.5. Add `GET /evals-be/api/evals/scores/:summary_id` handler returning all score rows for a summary grouped by metric. Rationale: minimal read API for dashboard and debugging.

- [x] Task 4.6. Add `GET /evals-be/api/evals/summary?eval_version=v1.0` aggregate endpoint returning `{ total_summaries, total_scored, per_metric: {metric -> {avg, min, max, count}} }`. Rationale: powers the dashboard panel; avoids the dashboard issuing 657 requests.

### Step 5 — LLM Groundedness Pipeline

- [x] Task 5.1. Add `evals-be/crates/services/src/groundedness.rs` with a `judge_groundedness(summary_id, config) -> (f32, f32, serde_json::Value)` function returning `(claim_groundedness, hallucination_rate, details_json)`. This function is wrapped by `cache::score_with_cache` in `run_eval.rs` so it only runs on a cache miss; the expensive LLM + retrieval work below is skipped entirely when the `(summary_hash, metric, eval_version)` row already exists. Internally on miss it:
  1. Loads the summary text and region name from DB
  2. POSTs `/brainatlas-be/api/llm/extract-claims` -> `ClaimsResponse`
  3. For each claim:
     a. POSTs `/brainatlas-be/api/llm/embed` with the claim text -> embedding
     b. Runs a pgvector similarity query against `brain_region_embeddings` filtered by `summary_id` (the claim must be grounded in *this* summary's source chunks, not any chunk for the region)
     c. Filters chunks above `EvalSimilarityThreshold`
     d. If no chunks above threshold -> claim is "unsupported" without calling the judge (saves tokens)
     e. Else POSTs `/brainatlas-be/api/llm/judge-groundedness` with the top-K chunks
  4. Aggregates verdicts: `groundedness = count(supported) / total`, `hallucination = count(unsupported) / total`
  5. Stores the full per-claim JSON array in `details` for drill-down
  Rationale: retrieval-filter-judge flow mirrors RAG best practices; the similarity prefilter cuts LLM cost by ~30% without hurting signal. The hash cache compounds this: a re-score of identical text pays zero LLM tokens.

- [x] Task 5.2. Extend `run_eval.rs::run_structural_metrics` into `run_all_metrics(summary_id)` that runs structural first (fast), then groundedness, then rubric (Step 6). Each metric writes its score row independently so a late failure doesn't lose earlier work. Rationale: progressive enhancement — if the rubric LLM times out, the groundedness score still persists.

- [x] Task 5.3. Add retry wrapper using `backon::ExponentialBuilder` around each brainatlas call (reuse pattern from `orch/crates/services/src/completion_watcher.rs:424-439`). Max 3 attempts, 1s-10s delay. Rationale: transient LLM provider errors must not kill an eval run.

- [x] Task 5.4. Add unit test for `groundedness.rs` with a mocked brainatlas client that returns canned responses, verifying the aggregation math. Rationale: aggregation formulas are specifications; they must be locked against silent drift.

- [x] Task 5.5. Add integration test `evals-be/crates/app/tests/cache_hit.rs`: insert a known `region_summary` row, call `POST /evals/score` twice, assert (a) both calls return identical score values, (b) the second call reports `cached: true` for every metric, (c) the mocked brainatlas client received LLM calls only on the first invocation. Rationale: the cache is a correctness feature, not an optimization — the test must prove the second run performs zero LLM work.

### Step 6 — LLM Rubric Pipeline

- [x] Task 6.1. Add `evals-be/crates/services/src/rubric.rs` with `judge_rubric(summary_id, config) -> HashMap<String, (f32, serde_json::Value)>`. Calls `/brainatlas-be/api/llm/judge-rubric` once, receives a `RubricScores` response, normalizes each 1-5 score to 0.0-1.0 via `(score - 1.0) / 4.0`, returns one entry per criterion. Rationale: one LLM call for five metrics is dramatically cheaper than five calls.

- [x] Task 6.2. Wire rubric into `run_all_metrics`. Write five score rows (`rubric_relevance`, `rubric_coherence`, `rubric_specificity`, `rubric_clinical_utility`, `rubric_terminology`), each with the per-criterion rationale stored in `details`. All five go through `cache::score_with_cache`, so a cache hit on any one short-circuits that metric independently — but because the five rubric sub-scores come from a single LLM call, the app layer should fetch them together: first probe all five cache keys, only invoke the rubric LLM if **any** is missing, then cache-insert the missing rows from the single LLM response. Rationale: storing rationale is critical for human review when a score looks suspicious; bundling the five keys behind one LLM call avoids paying 5x for a partial miss.

### Step 7 — Orch Phase-4 Eval Orchestrator

Orch schedules evals. Separate from the existing three phases so a broken evals service doesn't stall the main pipeline.

- [x] Task 7.1. Add `ConfigKey::EvalOrchestratorEnabled`, `ConfigKey::EvalOrchestratorPollIntervalSecs` (default 60), `ConfigKey::EvalOrchestratorConcurrency` (default 5), `ConfigKey::EvalsBaseUrl` to `orch/crates/domain/src/lib.rs`. Rationale: matches the pattern of other orch features being enable-able and concurrency-tunable from config.

- [x] Task 7.2. Add migration `orch/migrations/<timestamp>-add_eval_config/up.sql` seeding the new config keys with defaults. `EvalOrchestratorEnabled = 'false'` initially so the feature ships dark. Rationale: staged rollout — land the code behind a flag, enable after smoke-testing.

- [x] Task 7.3. Add `orch/crates/services/src/eval_orchestrator.rs` with a `poll` loop:
  1. Query active summaries in `region_summary` (from orch DB, which is the same DB as evals-be — both point to `appdb`)
  2. Left-join against `eval_runs` on `(summary_id, eval_version)` to find summaries with no run, or runs with `status = 'failed'` older than N hours
  3. For each candidate, POST `/evals-be/api/evals/score {summary_id, eval_version}` using `buffer_unordered(concurrency)`
  4. Evals-be writes the run row and score rows directly; orch just fires and waits for HTTP 200
  Rationale: polling avoids coupling — if evals-be is down, orch retries next cycle without any explicit retry logic.

- [x] Task 7.4. Add a fourth `tokio::spawn` block in `OrchApp::init()` at `orch/crates/app/src/app.rs` for the eval orchestrator loop, gated on `ConfigKey::EvalOrchestratorEnabled`. Rationale: matches the existing pattern for pipeline + monitor loops; enable/disable without redeploy.

- [x] Task 7.5. Expose `GET /orch/api/evals/status` that returns `{ total_summaries, queued, running, complete, failed }` by proxying to `evals-be/api/evals/summary`. Rationale: single health endpoint for the main orch dashboard.

### Step 8 — Dashboard Integration

- [DONE] Task 8.1. Extend `orch/crates/server/src/dev_stats.html` with a new "Evals" section showing:
  - Total summaries vs total scored (progress bar)
  - Per-metric aggregate table: metric | avg score | min | max | count
  - "Worst offenders" table: summaries with `claim_groundedness < 0.7`, joined against region name, with a link to view the summary
  Uses the existing `fetch` + `setInterval` pattern. Rationale: surfaces the eval signal where operators already look.

- [DONE] Task 8.2. Add tooltip descriptions for each metric in the `TIPS` map at `dev_stats.html:349-381`. Sample entries: `"Claim Groundedness": "Fraction of claims in the summary that a judge LLM verified against retrieved source chunks (0-1)."`, `"Hallucination Rate": "Fraction of claims with no supporting chunk above the similarity threshold."`. Rationale: metric semantics must be self-documenting for ops.

- [DONE] Task 8.3. Add a `GET /evals-be/api/evals/worst?metric=X&limit=N` endpoint returning the N lowest-scoring summaries for a given metric, joined with region name. Rationale: powers the worst-offenders table without dashboard-side filtering.

### Step 9 — Backfill + Acceptance

- [SKIPPED — operational/deploy step] Task 9.1. Run diesel migrations against production DB: `DATABASE_URL=... diesel migration run` in `evals-be/` and `orch/` to create the new tables and seed config keys. Rationale: schema must exist before the service starts.

- [SKIPPED — operational/deploy step] Task 9.2. Deploy all three services (brainatlas-be with new endpoints, evals-be new, orch with Phase 4). Keep `EvalOrchestratorEnabled = false`. Rationale: cold-start the services, verify container health, no eval work yet.

- [SKIPPED — requires running evals-be + DB] Task 9.3. Smoke-test evals-be manually: pick one summary UUID and POST `/evals-be/api/evals/score {summary_id: <uuid>}`. Verify all 10 metric rows appear in `eval_scores`. Rationale: end-to-end flow validation before enabling autonomous orchestration.

- [SKIPPED — requires running orch + DB] Task 9.4. Enable `EvalOrchestratorEnabled = true` via `PATCH /orch/api/config`. Watch `/dev/stats` Evals panel fill in over the next ~30 minutes as the 657 existing summaries are scored at concurrency 5 (~2 min/summary for LLM paths = ~4 hours total corpus; structural metrics finish in ~1 min). Rationale: gradual fill lets you catch failure modes (timeouts, judge model regressions) before committing to automation.

- [SKIPPED — requires running services + DB] Task 9.5. Verify the orchestrator picks up newly generated summaries automatically: run `curl -X POST /orch/api/regions/<uuid>/generate`, wait for the batch to complete, and within `EvalOrchestratorPollIntervalSecs` (60s) + eval runtime (~2 min) see the new summary appear in `eval_scores`. Rationale: confirms the fully autonomous loop works.

- [DONE] Task 9.6. Run `cargo check --workspace` in `orch/`, `brainatlas-be/`, and `evals-be/`. All three must pass with zero warnings. Rationale: final gate before merge. **Verified: all three workspaces build cleanly with `cargo check --workspace --all-targets` (zero warnings, zero errors).**

## Cost Estimate

Per-summary total LLM cost (claim extraction + ~30 claim judgments + rubric):
- Claim extraction: 1 call with cheap model (`gpt-4o-mini` ~$0.003)
- Claim embedding: ~30 embed calls (~$0.001)
- Groundedness judging: ~30 calls with cheap model (~$0.030)
- Rubric: 1 call with strong model (`gpt-4o` ~$0.010)
- **Per summary: ~$0.044**

For 657 summaries one-time backfill: **~$30**. Ongoing per new summary: **~$0.044**.

## Rollback

Every step is additive. To back out:
- Phase 4 orchestrator: set `EvalOrchestratorEnabled = false` in config (live, no redeploy).
- Evals-be service: stop the container. Orch's POSTs will fail (logged, not fatal). Main pipeline unaffected.
- Brainatlas endpoints: unused by existing clients, safe to leave in place.
- DB tables: `DROP TABLE eval_scores, eval_runs` (reversible via down migration) if a full reset is needed.

## Open Questions

1. **Which chat model for rubric judging?** `gpt-4o` is stronger but ~3x the cost of `gpt-4o-mini`. Recommendation: start with `gpt-4o-mini` for the backfill to stay under $20; switch to `gpt-4o` for incremental evals once the per-summary rate is low.
2. **~~Should we cache claim extraction?~~** Resolved: the v1 schema caches the final score by `summary_hash` (see Step 2 and Step 4.0), which subsumes the claim-extraction cache. If intermediate claim JSON is needed for offline analysis later, add an `eval_claim_cache(summary_hash PRIMARY KEY, claims JSONB)` table as an additive v2 change — the cache key is already established.
3. **Human-in-the-loop labeling?** For a small golden set (~50 regions), having a neuroscientist manually score summaries on the same rubric would let us correlate LLM-judge scores against ground truth. Out of scope for v1.
4. **Per-section scoring?** Section-level scores (e.g. groundedness of the "Associated Disorders" section specifically) would be more diagnostic than whole-summary scores. Out of scope for v1; the `section` field on each `Claim` enables this later without schema change.
5. **Cache invalidation strategy?** The hash-keyed cache is correct by construction — identical bytes produce identical scores — so invalidation only happens via `eval_version` bumps (e.g. prompt template changes, rubric rewording, judge-model swap). Operators must bump `EvalVersion` in config whenever any scoring logic changes; failure to do so would serve stale scores from the cache. Consider adding a CI check or startup assertion that hashes the prompt templates and refuses to boot if the template hash doesn't match the one recorded for the current `eval_version`. Out of scope for v1 but a known footgun.
