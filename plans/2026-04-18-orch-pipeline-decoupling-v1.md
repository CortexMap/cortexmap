# Decouple Orch Pipeline: Automatic Background Pipeline

## Objective

Refactor the orchestrator (`orch/`) pipeline from an **on-demand per-region** model (where `POST /regions/{id}/generate` triggers query generation + paper fetching inline) to an **automatic three-phase background pipeline** that runs on server startup and continuously keeps all regions up to date:

1. **Phase 1 — Generate queries** for all regions that need them (new or stale) and store them
2. **Phase 2 — Enqueue PMC ID fetch tasks** for all stored queries that haven't been fetched yet (reuses existing fetcher queue)
3. **Phase 3 — Run fetcher workers continuously** until the queue is drained

The pipeline runs automatically when the orch server starts. A configurable **summary staleness duration** (`SummaryMaxAgeSecs`, default 7 days) determines when a region's data is considered stale and should be regenerated. The existing `CompletionWatcher` background loop (which promotes `collecting` → `ready` → `processing` → `completed`) continues to run alongside.

### Future consideration (not in scope)

After this work, we plan to implement a **device-aware retry strategy** in the fetcher. Each worker will have a unique device ID. Since NCBI rate-limits by IP, workers will be distributed across multiple devices/IPs. When one device gets rate-limited, it goes on cooldown while others continue fetching. The current plan should **not** block this — the worker identity model (UUID-based `worker_id`) and the `ensure_workers_allocated` pattern are already compatible. Phase 3's worker health monitoring loop will naturally extend to device-level health tracking.

---

## Current Architecture (What Changes)

**Today** (`orch/crates/app/src/app.rs:266-398`): The `generate_summary(region_id)` method does everything inline for ONE region when a user calls `POST /regions/{id}/generate`:
- Generates LLM queries → stores them → creates a batch → enqueues fetch tasks → ensures workers → returns

**Today** (`orch/crates/app/src/app.rs:26-76`): The `init()` method spawns ONE background loop — the `CompletionWatcher` — that polls for batch completion and triggers brainatlas processing.

**After**: `init()` spawns a SECOND background loop — the **Pipeline Runner** — that automatically runs Phase 1 → Phase 2 → Phase 3 in a continuous cycle. Regions are processed based on staleness. The single-region `generate_summary` endpoint remains but becomes a "fast-path" that skips the queue for user-initiated requests.

**Key files impacted:**
- `orch/crates/domain/src/lib.rs` — New config keys (`SummaryMaxAgeSecs`, `PipelineBatchSize`, `PipelineCycleSleepSecs`)
- `orch/crates/domain/src/batch_types.rs` — New `PipelineProgress` type
- `orch/crates/app/src/services.rs` — New trait methods for each phase
- `orch/crates/app/src/app.rs` — New `run_pipeline_loop()` spawned from `init()`
- `orch/crates/services/src/region_management.rs` — Bulk query generation
- `orch/crates/services/src/batch_orchestration.rs` — Bulk enqueue
- `orch/crates/services/src/completion_watcher.rs` — Worker lifecycle monitoring for Phase 3
- `orch/crates/services/src/infra.rs` — New infra trait methods
- `orch/crates/infra/src/pg.rs` — New DB queries (stale regions, unenqueued queries, queue depth)
- `orch/migrations/` — New migration for config keys

---

## Implementation Plan

### Step 1: Domain Types and Config Keys

- [ ] **1.1 Add new config keys to `ConfigKey` enum in `orch/crates/domain/src/lib.rs`**
  - `SummaryMaxAgeSecs` — How old a completed summary can be before the region is considered stale and re-processed. Default: `604800` (7 days). Set to `0` to disable automatic re-processing of existing summaries (only process regions that have never been summarized).
  - `PipelineBatchSize` — How many regions to process concurrently in Phase 1 and Phase 2. Default: `5`. Controls LLM and NCBI API load.
  - `PipelineCycleSleepSecs` — How long to sleep between full pipeline cycles (after Phase 3 completes or the pipeline finds nothing to do). Default: `3600` (1 hour).

- [ ] **1.2 Add `PipelineProgress` type to `orch/crates/domain/src/api_types.rs`**
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct PipelineProgress {
      pub phase: String,                    // "idle", "generating_queries", "enqueuing_fetches", "fetching_papers"
      pub regions_needing_queries: usize,   // Phase 1 work-list size
      pub queries_generated: usize,         // Phase 1 progress
      pub regions_needing_enqueue: usize,   // Phase 2 work-list size
      pub queries_enqueued: usize,          // Phase 2 progress
      pub pending_fetch_tasks: i64,         // Phase 3 queue depth
      pub active_workers: usize,            // Phase 3 worker count
      pub cycle_count: u64,                 // How many full cycles have completed
      pub last_cycle_completed_at: Option<chrono::DateTime<chrono::Utc>>,
  }
  ```

- [ ] **1.3 Add config defaults via new migration in `orch/migrations/`**
  Create `orch/migrations/2026-04-18-000001-add_pipeline_config/up.sql`:
  ```sql
  INSERT INTO orch_config (key, value, description) VALUES
      ('summary_max_age_secs', '604800', 'Max age of a summary in seconds before region is re-processed (default 7 days, 0 = only new regions)'),
      ('pipeline_batch_size', '5', 'Number of regions to process concurrently in pipeline phases'),
      ('pipeline_cycle_sleep_secs', '3600', 'Sleep duration between pipeline cycles in seconds (default 1 hour)')
  ON CONFLICT (key) DO NOTHING;
  ```
  And corresponding `down.sql`:
  ```sql
  DELETE FROM orch_config WHERE key IN ('summary_max_age_secs', 'pipeline_batch_size', 'pipeline_cycle_sleep_secs');
  ```

### Step 2: Infrastructure Layer — New DB Queries

- [ ] **2.1 Add `get_regions_needing_queries` to `RegionMappingQueries` trait in `orch/crates/services/src/infra.rs`**
  Signature: `async fn get_regions_needing_queries(&self, database_url: &str, max_age_secs: i64) -> Result<Vec<RegionMapping>, Self::Error>`
  
  Returns regions from `region_mapping` that need fresh queries. A region needs queries if:
  - It has zero rows in `region_queries`, OR
  - Its most recent completed batch's `completed_at` is older than `NOW() - max_age_secs` (stale), OR
  - Its most recent batch is `failed` or `invalidated` (needs retry)
  
  Excludes regions that have an active batch (`collecting`, `ready`, or `processing`) — those are already in-flight.
  
  SQL sketch:
  ```sql
  SELECT rm.* FROM region_mapping rm
  WHERE NOT EXISTS (
      -- Exclude regions with active batches
      SELECT 1 FROM region_processing_batches rpb
      WHERE rpb.region_id = rm.id
      AND rpb.status IN ('collecting', 'ready', 'processing')
  )
  AND (
      -- No queries at all
      NOT EXISTS (SELECT 1 FROM region_queries rq WHERE rq.region_id = rm.id)
      OR
      -- Latest completed batch is stale
      (SELECT MAX(rpb.completed_at) FROM region_processing_batches rpb
       WHERE rpb.region_id = rm.id AND rpb.status = 'completed')
       < NOW() - INTERVAL '1 second' * $1
      OR
      -- Latest batch failed/invalidated and no active batch
      (SELECT rpb.status FROM region_processing_batches rpb
       WHERE rpb.region_id = rm.id ORDER BY rpb.created_at DESC LIMIT 1)
       IN ('failed', 'invalidated')
      OR
      -- Has queries but never had a batch (queries generated but never enqueued)
      (EXISTS (SELECT 1 FROM region_queries rq WHERE rq.region_id = rm.id)
       AND NOT EXISTS (SELECT 1 FROM region_processing_batches rpb WHERE rpb.region_id = rm.id))
  )
  ```

- [ ] **2.2 Add `get_regions_with_unenqueued_queries` to `RegionMappingQueries` trait in `orch/crates/services/src/infra.rs`**
  Signature: `async fn get_regions_with_unenqueued_queries(&self, database_url: &str) -> Result<Vec<(Uuid, Vec<String>)>, Self::Error>`
  
  Returns `(region_id, [query_text, ...])` tuples for regions that have `region_queries` rows but no active batch (no batch in `collecting`, `ready`, or `processing` status). These are the Phase 2 work-list — queries that have been generated but not yet sent to the fetcher.

- [ ] **2.3 Add `get_pending_fetch_task_count` to `BatchManagement` trait in `orch/crates/services/src/infra.rs`**
  Signature: `async fn get_pending_fetch_task_count(&self, database_url: &str) -> Result<i64, Self::Error>`
  
  Simple count: `SELECT COUNT(*) FROM fetch_tasks WHERE status IN ('pending', 'in_progress')`. Used by Phase 3 to know when the queue is drained.

- [ ] **2.4 Implement the three new methods in `orch/crates/infra/src/pg.rs`**
  Write the Diesel queries (or `diesel::sql_query` for the complex one in 2.1). All three are read-only queries with no side effects.

### Step 3: Service Layer — Phase Implementations

- [ ] **3.1 Add Phase 1 + Phase 2 + Phase 3 methods to service traits in `orch/crates/app/src/services.rs`**
  
  On `RegionManagement`:
  ```rust
  /// Phase 1: Generate queries for all regions that need them (new or stale)
  async fn generate_queries_for_stale_regions(&self) -> Result<PipelineProgress, Self::Error>;
  ```
  
  On `BatchOrchestration`:
  ```rust
  /// Phase 2: Create batches and enqueue fetch tasks for all regions with unenqueued queries
  async fn enqueue_all_pending_queries(&self) -> Result<PipelineProgress, Self::Error>;
  
  /// Phase 3: Get current queue depth (pending + in_progress fetch tasks)
  async fn get_pending_fetch_task_count(&self) -> Result<i64, Self::Error>;
  ```

- [ ] **3.2 Implement Phase 1 in `orch/crates/services/src/region_management.rs`**
  In `OrchRegionManagement`, implement `generate_queries_for_stale_regions`:
  1. Read `SummaryMaxAgeSecs` from config (default 604800)
  2. Read `PipelineBatchSize` from config (default 5)
  3. Read `QueryGenerationLimit` from config (default 3)
  4. Call `get_regions_needing_queries(max_age_secs)` to get the work-list
  5. If empty, return immediately with `phase: "idle"` progress
  6. Process regions in chunks of `PipelineBatchSize` **sequentially** (to respect LLM rate limits):
     - For each region: call existing `generate_queries(region_name, count)` → `insert_queries(region_id, queries)`
     - If the region already has queries (stale re-run), delete old queries first via `delete_queries(region_id)`
     - Log per-region success/failure, continue on error
  7. Return `PipelineProgress` with counts
  
  **Reuses**: `generate_queries()` (brainatlas HTTP), `insert_queries()` (DB), `delete_queries()` (DB), `get_region_name()` (DB)

- [ ] **3.3 Implement Phase 2 in `orch/crates/services/src/batch_orchestration.rs`**
  In `OrchBatchOrchestration`, implement `enqueue_all_pending_queries`:
  1. Call `get_regions_with_unenqueued_queries()` to get the work-list
  2. Read `PipelineBatchSize` from config (default 5)
  3. For each `(region_id, queries)` group, sequentially:
     a. `create_batch(region_id, queries.len())` (existing)
     b. For each query: `enqueue_fetch_task(query, region_id, Priority::Background)` (existing — calls fetcher `/enqueue` which does ESearch)
     c. Collect `task_ids`, `add_tasks_to_batch(batch_id, task_ids)` (existing)
     d. If no tasks created: `update_batch_status(batch_id, Failed, "No papers found")` (existing pattern)
     e. Log per-region results, continue on error
  4. Return `PipelineProgress` with counts
  
  **Reuses**: `create_batch()`, `enqueue_fetch_task()`, `add_tasks_to_batch()`, `update_batch_status()`, `update_batch_expected_count()` — all existing methods.
  
  Also implement `get_pending_fetch_task_count` — simple delegation to infra.

### Step 4: Application Layer — Background Pipeline Runner

- [ ] **4.1 Add `run_pipeline_loop` to `OrchApp` in `orch/crates/app/src/app.rs`**
  This is the core new method. It runs as a background `tokio::spawn` task from `init()`, alongside the existing `CompletionWatcher` loop.
  
  ```rust
  async fn run_pipeline_loop(services: Arc<S>) {
      // Small initial delay to let the server finish starting
      tokio::time::sleep(Duration::from_secs(10)).await;
      tracing::info!("Pipeline runner started");
      
      let mut cycle_count: u64 = 0;
      
      loop {
          tracing::info!(cycle = cycle_count, "Starting pipeline cycle");
          
          // === Phase 1: Generate queries for stale/new regions ===
          match services.generate_queries_for_stale_regions().await {
              Ok(progress) => {
                  tracing::info!(
                      regions = progress.regions_needing_queries,
                      generated = progress.queries_generated,
                      "Phase 1 complete: query generation"
                  );
              }
              Err(e) => tracing::error!(error = ?e, "Phase 1 failed"),
          }
          
          // === Phase 2: Enqueue fetch tasks for unenqueued queries ===
          match services.enqueue_all_pending_queries().await {
              Ok(progress) => {
                  tracing::info!(
                      regions = progress.regions_needing_enqueue,
                      enqueued = progress.queries_enqueued,
                      "Phase 2 complete: fetch enqueue"
                  );
              }
              Err(e) => tracing::error!(error = ?e, "Phase 2 failed"),
          }
          
          // === Phase 3: Ensure workers are running and wait for queue drain ===
          if let Err(e) = services.ensure_workers_allocated().await {
              tracing::warn!(error = ?e, "Failed to ensure workers allocated");
          }
          
          // Monitor queue until empty
          loop {
              match services.get_pending_fetch_task_count().await {
                  Ok(count) if count > 0 => {
                      tracing::info!(pending = count, "Phase 3: fetch queue not empty, waiting...");
                      tokio::time::sleep(Duration::from_secs(30)).await;
                  }
                  Ok(_) => {
                      tracing::info!("Phase 3 complete: fetch queue drained");
                      break;
                  }
                  Err(e) => {
                      tracing::error!(error = ?e, "Phase 3: failed to check queue depth");
                      break;
                  }
              }
              
              // Re-check worker health periodically
              if let Err(e) = services.ensure_workers_allocated().await {
                  tracing::warn!(error = ?e, "Failed to re-ensure workers during Phase 3");
              }
          }
          
          cycle_count += 1;
          
          // Sleep between cycles
          let sleep_secs = match services.get_config(ConfigKey::PipelineCycleSleepSecs).await {
              Ok(Some(v)) => v.parse::<u64>().unwrap_or(3600),
              _ => 3600,
          };
          tracing::info!(cycle = cycle_count, sleep_secs, "Pipeline cycle complete, sleeping");
          tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
      }
  }
  ```

- [ ] **4.2 Spawn the pipeline runner from `init()` in `orch/crates/app/src/app.rs`**
  In the existing `init()` method (which currently only spawns the `CompletionWatcher` loop), add a second `tokio::spawn` for `run_pipeline_loop`. Both loops run concurrently:
  - **CompletionWatcher loop** (existing): Polls every 30s, promotes `collecting` → `ready` → `processing` → `completed` batches, calls brainatlas `/process`
  - **Pipeline Runner loop** (new): Runs Phase 1 → 2 → 3, sleeps `PipelineCycleSleepSecs`, repeats
  
  These two loops are complementary:
  - Pipeline Runner creates batches (Phase 2) and fills the fetch queue (Phase 3)
  - CompletionWatcher detects when batches are `ready` and triggers LLM processing

### Step 5: Refactor `generate_summary` to Reuse Stored Queries

- [ ] **5.1 Modify `generate_summary` in `orch/crates/app/src/app.rs` to check for existing queries**
  Currently `generate_summary` always calls the LLM to generate queries. After this change:
  1. Check `services.get_queries(region_id)` first
  2. If queries exist and are non-empty, use them directly (skip LLM call)
  3. Only call `services.generate_queries(region_name, count)` if no stored queries exist
  
  This makes the user-triggered endpoint benefit from queries pre-generated by the pipeline, and avoids redundant LLM calls. ~5-10 lines changed.

### Step 6: Pipeline Status Endpoint

- [ ] **6.1 Add `GET /orch/api/pipeline/progress` endpoint**
  Store `PipelineProgress` in an `Arc<RwLock<PipelineProgress>>` shared between the pipeline runner and the server. The pipeline runner updates it at each phase transition. The endpoint returns the current snapshot.
  
  Changes needed:
  - Add `pipeline_progress: Arc<RwLock<PipelineProgress>>` field to `OrchServer`
  - Pass it through to `OrchApp::init()` so the spawned task can write to it
  - Add handler + route in `server.rs`
  - Add trait method in `OrchApi` + impl in `orch_api.rs`

---

## Verification Criteria

- On server startup, the pipeline runner automatically starts after a 10s delay
- Phase 1 finds regions that have never been summarized, or whose latest summary is older than `SummaryMaxAgeSecs`, and generates queries for them
- Phase 1 is idempotent: re-running skips regions that already have fresh queries and no stale summary
- Phase 2 creates batches and enqueues fetch tasks for all regions with queries but no active batch
- Phase 2 is idempotent: re-running skips regions that already have an active batch
- Phase 3 starts workers (via existing `ensure_workers_allocated`), monitors queue depth, and waits until `fetch_tasks` has zero pending/in_progress rows
- The existing `CompletionWatcher` loop continues to work alongside — it promotes completed batches and triggers brainatlas processing
- The existing `generate_summary` single-region endpoint still works, and reuses pre-generated queries when available
- `GET /orch/api/pipeline/progress` returns current phase and progress counters
- Partial failures in Phase 1/2 don't block other regions (log + continue pattern)
- Setting `SummaryMaxAgeSecs = 0` disables staleness-based re-processing (only processes regions that have never been summarized)
- The pipeline sleeps `PipelineCycleSleepSecs` between full cycles to avoid continuous load

---

## Interaction Between Pipeline Runner and CompletionWatcher

```
Pipeline Runner (new)                    CompletionWatcher (existing)
═══════════════════                      ═══════════════════════════
                                         
Phase 1: Generate queries                Polls every 30s:
  → region_queries rows created          
                                         
Phase 2: Enqueue fetches                 For each batch in "collecting":
  → region_processing_batches created      Check if all fetch_task_ids complete
    (status = "collecting")                If yes → mark batch "ready"
  → fetch_tasks created via fetcher      
                                         For each batch in "ready":
Phase 3: Wait for queue drain              Call brainatlas /process
  → Workers process fetch_tasks            Mark batch "processing" → "completed"
  → fetch_tasks status → "completed"     
                                         ← CompletionWatcher picks up the batch
                                           and triggers LLM summarization
                                         
Sleep PipelineCycleSleepSecs             (continues polling)
Repeat                                   
```

---

## Potential Risks and Mitigations

1. **LLM cost on startup**
   If `region_mapping` has 1000+ regions and none have been processed, Phase 1 will generate 3000+ LLM queries on first startup.
   *Mitigation*: `PipelineBatchSize` (default 5) limits concurrency. Processing is sequential within each batch. The pipeline can be interrupted and will resume where it left off (idempotent). Set `SummaryMaxAgeSecs = 0` to only process new regions.

2. **NCBI rate limiting in Phase 2**
   Each query triggers an ESearch call. Many queries at once could trigger rate limits.
   *Mitigation*: `PipelineBatchSize` limits concurrency. The fetcher already has request-level retry with exponential backoff (`fetcher-be/crates/cortexmap-fetcher/src/retry.rs:87-112`). Future device-aware retry strategy will further mitigate this.

3. **Race condition between pipeline runner and user-triggered `generate_summary`**
   If the pipeline is processing region X in Phase 2 and a user simultaneously calls `generate_summary` for region X.
   *Mitigation*: The existing active-batch guard in `generate_summary` (`app.rs:270-283`) returns the existing batch if one is active. The `UNIQUE` partial index on `region_processing_batches` (`idx_one_active_batch_per_region`) prevents duplicate active batches at the DB level.

4. **Pipeline runner and CompletionWatcher stepping on each other**
   Both run as background loops accessing the same tables.
   *Mitigation*: They operate on different batch statuses. Pipeline Runner creates `collecting` batches; CompletionWatcher promotes `collecting` → `ready` → `processing` → `completed`. No overlap in writes. All DB operations use proper transactions.

5. **Startup storm after deployment**
   If many orch instances start simultaneously (horizontal scaling), each would try to run the pipeline.
   *Mitigation*: The active-batch guard and `UNIQUE` partial index prevent duplicate batches. Multiple instances running Phase 1 would generate duplicate queries, but `insert_queries` is append-only and the next Phase 2 run would create one batch per region (guarded by the active-batch check). For production, consider a leader-election mechanism (e.g., PostgreSQL advisory lock) — but this is out of scope for v1.

6. **Worker lifecycle in Phase 3**
   Workers might crash or get rate-limited during long queue processing.
   *Mitigation*: Phase 3's monitoring loop periodically calls `ensure_workers_allocated()` to restart dead workers. The fetcher has stale task recovery (`reset_stale_tasks`). Future device-aware retry will add per-device cooldown.
