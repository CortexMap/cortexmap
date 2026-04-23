# Decouple Orch Pipeline: Batch Query Generation, ID Fetching, and Continuous Worker Execution

## Objective

Refactor the orchestrator (`orch/`) pipeline from an **on-demand per-region** model (where `POST /regions/{id}/generate` triggers query generation + paper fetching inline) to a **three-phase batch pipeline**:

1. **Phase 1 — Generate queries for ALL regions** and store them
2. **Phase 2 — Enqueue PMC ID fetch tasks** for all stored queries (using existing fetcher queue)
3. **Phase 3 — Run fetcher workers continuously** until the queue is drained

The current single-region `generate_summary` flow remains available but the new pipeline becomes the primary orchestration mode.

**Forward-looking constraint**: A subsequent iteration will introduce **device-aware retry** where each worker carries a `device_id` (representing a physical machine/IP). When NCBI rate-limits a device (HTTP 429), all workers on that device enter cooldown while workers on other devices continue. The current plan must not create obstacles for this and should lay groundwork where cheap to do so.

---

## Current Architecture (What Changes)

**Today** (`orch/crates/app/src/app.rs:266-398`): The `generate_summary(region_id)` method does everything inline for ONE region:
- Generates LLM queries -> stores them -> creates a batch -> enqueues fetch tasks -> ensures workers -> returns

**After**: These responsibilities are separated into distinct phases that can run independently, operating on ALL regions at once.

**Key files impacted (orch/):**

| Layer | File | Change |
|-------|------|--------|
| Domain | `crates/domain/src/lib.rs` | New types: `PipelinePhase`, progress structs, config keys |
| Infra trait | `crates/services/src/infra.rs` | New methods on `RegionMappingQueries` and `BatchManagement` |
| Infra impl | `crates/infra/src/pg.rs` | Diesel queries for bulk operations |
| Service | `crates/services/src/region_management.rs` | Bulk query generation |
| Service | `crates/services/src/batch_orchestration.rs` | Bulk enqueue |
| Service | `crates/services/src/completion_watcher.rs` | Continuous fetch monitoring |
| App | `crates/app/src/services.rs` | New trait methods |
| App | `crates/app/src/app.rs` | Pipeline orchestration |
| API | `crates/api/src/orch_api.rs` | New API methods |
| Server | `crates/server/src/server.rs` | New endpoints |
| Migration | `migrations/` | New config key defaults |

**Key files impacted (fetcher-be/) — forward-looking only, no changes in this iteration:**

| File | Future Change (device-aware retry) |
|------|--------------------------------------|
| `crates/cortexmap-be/src/worker_manager.rs` | `allocate_workers` will accept `device_id` param |
| `crates/cortexmap-fetcher/src/worker.rs` | Worker loop will propagate 429 to device-level cooldown |
| `crates/cortexmap-fetcher/src/retry.rs` | New `DeviceCooldownTracker` shared across workers on same device |
| `crates/cortexmap-core/src/blueprint/connections/fetcher.rs` | `Fetcher` struct gets `device_id` field |
| `migrations/` | `fetch_tasks` gets `device_id` column; new `device_cooldowns` table |

---

## Implementation Plan

### Step 1: Domain Types and Config

- [ ] **1.1 Add pipeline phase types to `orch/crates/domain/src/lib.rs`**
  Create a new file `orch/crates/domain/src/pipeline_types.rs` and re-export from `lib.rs`. Define:
  ```
  PipelinePhase { Idle, GeneratingQueries, EnqueueingFetches, FetchingPapers }
  QueryGenerationProgress { total_regions, completed, failed, skipped (already have queries) }
  BulkEnqueueProgress { total_queries, regions_processed, tasks_created, failed_queries }
  ContinuousFetchStatus { queue_depth, active_workers, completed_since_start, is_running }
  PipelineStatus { phase, phase_progress (serde_json::Value), started_at, updated_at }
  ```
  All derive `Serialize, Deserialize, Clone, Debug`.

- [ ] **1.2 Add config keys to `ConfigKey` enum in `orch/crates/domain/src/lib.rs`**
  Add three new variants:
  - `BulkQueryConcurrency` — max concurrent LLM query-generation calls (default: 5). Controls how many regions are processed in parallel during Phase 1.
  - `BulkEnqueueConcurrency` — max concurrent fetcher enqueue calls (default: 10). Controls how many queries are sent to the fetcher in parallel during Phase 2.
  - `ContinuousFetchPollSecs` — how often Phase 3 checks queue depth and worker health (default: 30).

- [ ] **1.3 Create migration for new config defaults**
  New migration in `orch/migrations/` that inserts:
  ```sql
  INSERT INTO orch_config (key, value, description) VALUES
    ('bulk_query_concurrency', '5', 'Max concurrent LLM calls during bulk query generation'),
    ('bulk_enqueue_concurrency', '10', 'Max concurrent fetcher enqueue calls during bulk enqueue'),
    ('continuous_fetch_poll_secs', '30', 'Poll interval for continuous fetch monitoring')
  ON CONFLICT (key) DO NOTHING;
  ```

### Step 2: Infrastructure Layer — New DB Queries

- [ ] **2.1 Add `get_regions_without_queries` to `RegionMappingQueries` trait (`orch/crates/services/src/infra.rs`)**
  Signature: `async fn get_regions_without_queries(&self, database_url: &str) -> Result<Vec<RegionMapping>, Self::Error>`
  Returns all `region_mapping` rows that have zero corresponding rows in `region_queries`.

- [ ] **2.2 Implement `get_regions_without_queries` in `orch/crates/infra/src/pg.rs`**
  Diesel query using `LEFT JOIN region_queries ON region_mapping.id = region_queries.region_id WHERE region_queries.id IS NULL`. This gives the work-list for Phase 1: regions that still need queries generated.

- [ ] **2.3 Add `get_unenqueued_queries` to `BatchManagement` trait (`orch/crates/services/src/infra.rs`)**
  Signature: `async fn get_unenqueued_queries(&self, database_url: &str) -> Result<Vec<(Uuid /*query_id*/, Uuid /*region_id*/, String /*query_text*/)>, Self::Error>`
  Returns `region_queries` rows for regions that do NOT have an active or completed batch (i.e., no row in `region_processing_batches` with `status IN ('collecting', 'ready', 'processing', 'completed')`). These are queries that still need to be sent to the fetcher.

- [ ] **2.4 Implement `get_unenqueued_queries` in `orch/crates/infra/src/pg.rs`**
  Query: select from `region_queries rq` where `NOT EXISTS (SELECT 1 FROM region_processing_batches rpb WHERE rpb.region_id = rq.region_id AND rpb.status IN ('collecting', 'ready', 'processing', 'completed'))`. Group by `region_id` for the service layer to process per-region.

- [ ] **2.5 Add `get_pending_fetch_task_count` to `BatchManagement` trait (`orch/crates/services/src/infra.rs`)**
  Signature: `async fn get_pending_fetch_task_count(&self, database_url: &str) -> Result<i64, Self::Error>`
  Counts `fetch_tasks WHERE status IN ('pending', 'in_progress')`. Used by Phase 3 to know when the queue is drained.

- [ ] **2.6 Implement `get_pending_fetch_task_count` in `orch/crates/infra/src/pg.rs`**
  Simple Diesel count query on `fetch_tasks` table.

### Step 3: Service Layer — Phase Implementations

- [ ] **3.1 Add Phase 1 trait method to `RegionManagement` in `orch/crates/app/src/services.rs`**
  ```rust
  async fn generate_queries_for_all_regions(&self) -> Result<QueryGenerationProgress, Self::Error>;
  ```

- [ ] **3.2 Implement Phase 1 in `orch/crates/services/src/region_management.rs`**
  In `OrchRegionManagement`, implement `generate_queries_for_all_regions`:
  1. Call `infra.get_regions_without_queries()` to get the work-list
  2. Read `BulkQueryConcurrency` from config (default 5)
  3. Read `QueryGenerationLimit` from config (default 3)
  4. Use `futures::stream::iter(regions).map(|region| async { ... }).buffer_unordered(concurrency)` to process in parallel
  5. For each region: call existing `generate_queries(&region.name, count)` then `insert_queries(region.id, queries)`
  6. Collect results, count successes/failures/skipped
  7. Return `QueryGenerationProgress`
  
  **Key reuse**: The inner loop calls the same `generate_queries()` method (`region_management.rs:179-243`) and `insert_queries()` that the single-region flow uses. No new HTTP or DB code for the inner operations.

- [ ] **3.3 Add Phase 2 trait method to `BatchOrchestration` in `orch/crates/app/src/services.rs`**
  ```rust
  async fn enqueue_all_queries(&self) -> Result<BulkEnqueueProgress, Self::Error>;
  ```

- [ ] **3.4 Implement Phase 2 in `orch/crates/services/src/batch_orchestration.rs`**
  In `OrchBatchOrchestration`, implement `enqueue_all_queries`:
  1. Call `infra.get_unenqueued_queries()` to get work-list
  2. Group by `region_id` (collect into `HashMap<Uuid, Vec<(Uuid, String)>>`)
  3. Read `BulkEnqueueConcurrency` from config (default 10)
  4. For each region group (sequentially or with bounded concurrency):
     a. `create_batch(region_id, query_count)` — existing method
     b. For each query: `enqueue_fetch_task(query_text, region_id, Priority::Background)` — existing method. This calls `POST /fetcher-be/api/queue/enqueue` which does ESearch + task creation.
     c. Collect all `task_ids`, call `add_tasks_to_batch(batch_id, task_ids)` — existing method
     d. If no tasks created, `update_batch_status(batch_id, Failed, "No papers found")` — existing pattern from `app.rs:374-385`
  5. Return `BulkEnqueueProgress`
  
  **Key reuse**: Entire inner loop uses existing `enqueue_fetch_task` (`batch_orchestration.rs:209-268`), `create_batch`, `add_tasks_to_batch`. The fetcher's `/enqueue` endpoint already handles ESearch -> PMC ID discovery -> task creation.

- [ ] **3.5 Add Phase 3 trait method to `CompletionOrchestrator` in `orch/crates/app/src/services.rs`**
  ```rust
  async fn run_continuous_fetch(&self) -> Result<ContinuousFetchStatus, Self::Error>;
  ```

- [ ] **3.6 Implement Phase 3 in `orch/crates/services/src/completion_watcher.rs`**
  In `CompletionWatcher`, implement `run_continuous_fetch`:
  1. Call `ensure_workers_allocated()` — existing method from `batch_orchestration.rs:341-460`
  2. Read `ContinuousFetchPollSecs` from config (default 30)
  3. Enter monitoring loop:
     a. `get_pending_fetch_task_count()` — new method from step 2.5
     b. If count > 0: log progress, check worker health via existing `get_worker_status()`. If all workers dead, re-allocate via `ensure_workers_allocated()`. Sleep `ContinuousFetchPollSecs`. Continue.
     c. If count == 0: queue drained. Return `ContinuousFetchStatus { is_running: false, queue_depth: 0, ... }`
  
  **Note**: This method blocks until the queue is empty. It should be spawned as a background task, not called from an HTTP handler directly. The HTTP endpoint triggers it and returns immediately; status is queried separately.

### Step 4: Application Layer — Pipeline Orchestration

- [ ] **4.1 Add pipeline methods to `OrchApp` in `orch/crates/app/src/app.rs`**
  Three new public methods:
  - `generate_all_queries(&self) -> Result<QueryGenerationProgress, E>` — delegates to `services.generate_queries_for_all_regions()`
  - `enqueue_all_fetches(&self) -> Result<BulkEnqueueProgress, E>` — delegates to `services.enqueue_all_queries()`, then calls `services.ensure_workers_allocated()`
  - `start_continuous_fetch(&self) -> Result<(), E>` — spawns `services.run_continuous_fetch()` as a `tokio::spawn` background task. Stores the `JoinHandle` for status tracking.

- [ ] **4.2 Add pipeline status tracking to `OrchApp`**
  Add a field `pipeline_status: Arc<RwLock<PipelineStatus>>` to `OrchApp` (or to `OrchServer` in the server crate — whichever holds mutable state).
  - Each phase method updates this status before starting, during progress, and on completion
  - `get_pipeline_status(&self) -> PipelineStatus` reads the current state
  - This enables the `GET /pipeline/status` endpoint without blocking

- [ ] **4.3 Optionally add `run_full_pipeline` convenience method**
  Chains Phase 1 -> Phase 2 -> Phase 3 sequentially. Updates pipeline status at each transition. Returns final status. Spawned as a background task.

### Step 5: API Layer — New Endpoints

- [ ] **5.1 Add pipeline methods to `OrchApi` trait (`orch/crates/api/src/lib.rs`)**
  ```rust
  async fn generate_all_queries(&self) -> Result<QueryGenerationProgress, Self::Error>;
  async fn enqueue_all_fetches(&self) -> Result<BulkEnqueueProgress, Self::Error>;
  async fn start_continuous_fetch(&self) -> Result<(), Self::Error>;
  async fn get_pipeline_status(&self) -> Result<PipelineStatus, Self::Error>;
  ```

- [ ] **5.2 Implement in `orch/crates/api/src/orch_api.rs`**
  Each delegates to `OrchApp` with the standard error mapping pattern (same as existing methods like `generate_summary` at `orch_api.rs:49-57`).

- [ ] **5.3 Add HTTP handlers and routes in `orch/crates/server/src/server.rs`**
  New routes under `/orch/api/pipeline/`:
  | Method | Path | Handler | Behavior |
  |--------|------|---------|----------|
  | `POST` | `/api/pipeline/generate-queries` | `generate_all_queries_handler` | Kicks off Phase 1 as background task, returns immediately with `{ "started": true }` |
  | `POST` | `/api/pipeline/enqueue-fetches` | `enqueue_all_fetches_handler` | Kicks off Phase 2 as background task, returns immediately |
  | `POST` | `/api/pipeline/start-fetch` | `start_continuous_fetch_handler` | Kicks off Phase 3 as background task, returns immediately |
  | `POST` | `/api/pipeline/run` | `run_full_pipeline_handler` | Kicks off Phase 1->2->3 as background task |
  | `GET` | `/api/pipeline/status` | `get_pipeline_status_handler` | Returns current `PipelineStatus` (phase, progress, timing) |

### Step 6: Refactor Existing `generate_summary`

- [ ] **6.1 Make `generate_summary` reuse pre-generated queries**
  In `orch/crates/app/src/app.rs:generate_summary` (around line 295-308), before calling `services.generate_queries()`, check if queries already exist:
  ```
  let existing_queries = self.services.get_queries(region_id).await?;
  let queries = if !existing_queries.is_empty() {
      existing_queries.iter().map(|q| q.query_text.clone()).collect()
  } else {
      // Generate fresh queries via LLM (existing path)
      self.services.generate_queries(&region_name, query_count).await?
  };
  ```
  This makes the single-region endpoint benefit from Phase 1's pre-generated queries without duplicating LLM calls.

### Step 7: Forward-Looking Groundwork for Device-Aware Retry

These items prepare the codebase for the next iteration (device-based rate-limit cooldown) without implementing it. They are **low-cost structural preparations** only.

- [ ] **7.1 Add `device_id` field to `AllocateWorkersRequest` in `orch/crates/domain/src/worker_types.rs`**
  Add an `Option<String>` field `device_id` to the existing `AllocateWorkersRequest` struct. Default `None`. The orch's worker allocation proxy (`batch_orchestration.rs:531-619`) will pass this through to the fetcher. No behavior change — just plumbing.
  
  **Rationale**: When device-aware retry is implemented, the orch will allocate workers per-device (e.g., "allocate 3 workers on device `proxy-us-east`"). Having the field already in the request type avoids a breaking API change later.

- [ ] **7.2 Add `device_id` field to `FetcherRetryConfig` in `orch/crates/domain/src/worker_types.rs`**
  Add `device_cooldown_secs: Option<u64>` — how long a device pauses after a 429. Default `None` (disabled). Passed through to fetcher but ignored until the fetcher implements the cooldown logic.
  
  **Rationale**: The orch already sends `FetcherRetryConfig` to the fetcher on every `allocate_workers` call (`batch_orchestration.rs:421-426`). Adding the field now means the fetcher can read it when the cooldown logic is implemented, without needing an orch release.

- [ ] **7.3 Ensure Phase 3 monitoring loop tracks per-worker status**
  In the `run_continuous_fetch` implementation (step 3.6), when checking worker health, store the full `Vec<WorkerStatus>` in the pipeline status. This makes it easy to add per-device grouping later.
  
  **Rationale**: The future device-aware system needs to know which workers are on which device. If the monitoring loop already tracks individual worker status, adding device grouping is a small change.

---

## Verification Criteria

- Phase 1 generates queries for all regions that don't already have them; is idempotent (re-running skips regions with existing queries)
- Phase 2 creates batches and enqueues fetch tasks for all stored queries without active/completed batches; is idempotent
- Phase 3 starts workers, monitors queue depth, re-allocates dead workers, and returns only when `fetch_tasks` has zero pending/in-progress rows
- The existing `generate_summary` single-region endpoint continues to work and reuses pre-generated queries
- The existing `CompletionWatcher` background loop continues to promote `collecting` -> `ready` -> `processing` -> `completed` batches (this is orthogonal to the new pipeline)
- Pipeline status is queryable via `GET /pipeline/status` at any time
- All three phases can be triggered independently via separate endpoints
- Partial failures in Phase 1/2 don't block other regions (error-and-continue pattern)
- New `device_id` and `device_cooldown_secs` fields are present in domain types but have no behavioral effect yet

---

## Potential Risks and Mitigations

1. **LLM cost explosion in Phase 1**
   If `region_mapping` has 1000+ regions, generating 3 queries each means 1000+ LLM calls.
   *Mitigation*: `BulkQueryConcurrency` config (default 5) limits parallelism. Phase 1 is idempotent — can be interrupted and resumed. Consider adding a `limit` parameter to the endpoint for partial runs.

2. **NCBI rate limiting in Phase 2**
   Each query triggers an ESearch call to NCBI. Thousands of queries could trigger rate limits.
   *Mitigation*: `BulkEnqueueConcurrency` (default 10) limits parallelism. The fetcher already retries on 429 with backoff (`retry.rs:29-38`). The inter-task `task_timeout_secs` delay in the worker loop (`worker.rs:498`) provides natural throttling. The future device-aware retry will further address this.

3. **Database contention during bulk operations**
   Mass inserts into `region_queries`, `region_processing_batches`, and `fetch_tasks`.
   *Mitigation*: Bounded concurrency via config. The fetcher's `FOR UPDATE SKIP LOCKED` (`std-infra/src/task_queue.rs:106-137`) handles worker contention. Batch inserts could be used for `region_queries` if contention is observed.

4. **Long-running HTTP requests timing out**
   Phase 1 for many regions could take hours.
   *Mitigation*: All phase endpoints return immediately after spawning background tasks. Progress is tracked via `Arc<RwLock<PipelineStatus>>` and queried via `GET /pipeline/status`.

5. **Existing `generate_summary` conflicts with bulk batches**
   Phase 2 creates batches per-region. If a user then calls `generate_summary` for the same region, the active-batch guard (`app.rs:270-283`) returns the existing batch.
   *Mitigation*: This is correct behavior — no change needed.

6. **Worker lifecycle in Phase 3**
   Workers might crash, stall, or all get rate-limited simultaneously.
   *Mitigation*: Phase 3 monitoring loop checks worker health on each poll iteration and re-allocates dead workers. The fetcher's `reset_stale_tasks` (`worker.rs:516-529`) recovers stuck tasks. The future device-aware retry will make this more sophisticated.

7. **CompletionWatcher vs. Phase 3 overlap**
   The existing `CompletionWatcher` polls for batch completion and triggers brainatlas processing. Phase 3 monitors queue depth. They run concurrently.
   *Mitigation*: They operate on different concerns — Phase 3 ensures fetch workers run; CompletionWatcher handles post-fetch processing. No conflict. Both should log clearly to distinguish their activities.

---

## Alternative Approaches

1. **Cron-based instead of API-triggered**: Run Phase 1->2->3 on a schedule. Simpler but less controllable and risks surprise LLM costs. Could be added as a config option later (`auto_pipeline_enabled`, `auto_pipeline_cron`).

2. **Streaming progress instead of polling**: Use SSE (Server-Sent Events) for real-time pipeline progress instead of `GET /pipeline/status` polling. More responsive but adds complexity. Could be added later.

3. **Separate pipeline service**: New microservice instead of extending orch. Cleaner separation but adds deployment complexity. Not recommended — orch is already the coordinator.

4. **Cursor-based partial phases**: Instead of "process all", endpoints accept `limit` + `offset` parameters. Client calls repeatedly. More resilient to timeouts but pushes orchestration to the client. Could be useful for Phase 1 specifically where LLM costs are a concern.

---

## Architecture Diagram: Before and After

### Before (current)
```
User -> POST /regions/{id}/generate
          |
          v
     [generate_summary]
     1. generate_queries(region_name) ----> brainatlas LLM
     2. store_queries(region_id)
     3. create_batch(region_id)
     4. for query: enqueue_fetch_task() --> fetcher /enqueue --> NCBI ESearch
     5. ensure_workers_allocated()     --> fetcher /workers/allocate
     6. return batch_id
          |
     [CompletionWatcher background loop]
     polls batches -> when ready -> call brainatlas /process
```

### After (new)
```
POST /pipeline/generate-queries (or /pipeline/run for all three)
  |
  v
[Phase 1: generate_queries_for_all_regions]
  for each region without queries:
    generate_queries(region_name) ----> brainatlas LLM
    store_queries(region_id)
  |
  v
POST /pipeline/enqueue-fetches
  |
  v
[Phase 2: enqueue_all_queries]
  for each region with unenqueued queries:
    create_batch(region_id)
    for query: enqueue_fetch_task() --> fetcher /enqueue --> NCBI ESearch
    add_tasks_to_batch()
  |
  v
POST /pipeline/start-fetch
  |
  v
[Phase 3: run_continuous_fetch]
  ensure_workers_allocated()
  loop:
    check queue depth
    check worker health (re-allocate if dead)
    if queue empty: done
    sleep(poll_interval)
  |
  v (in parallel, unchanged)
[CompletionWatcher background loop]
  polls batches -> when ready -> call brainatlas /process

GET /pipeline/status  <-- query progress at any time
```

### Future (device-aware retry, next iteration)
```
[Phase 3 enhanced]
  allocate_workers(device_id="proxy-us-east", count=3)
  allocate_workers(device_id="proxy-eu-west", count=3)
  loop:
    check queue depth
    for each device:
      if device.cooldown_active: skip
      check workers on device
      if workers dead: re-allocate on that device
    if queue empty: done

[Worker loop enhanced]
  on 429 response:
    notify DeviceCooldownTracker(device_id)
    all workers on same device_id pause for device_cooldown_secs
    workers on other devices continue
```
