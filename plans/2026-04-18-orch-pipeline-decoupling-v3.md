# Decouple Orch Pipeline: Batch Query Generation, ID Fetching, and Continuous Device-Orchestrated Execution

## Objective

Refactor the orchestrator (`orch/`) pipeline from an **on-demand per-region** model (where `POST /regions/{id}/generate` triggers query generation + paper fetching inline) to a **three-phase batch pipeline**:

1. **Phase 1 — Generate queries for ALL regions** and store them
2. **Phase 2 — Enqueue PMC ID fetch tasks** for all stored queries (using existing fetcher queue)
3. **Phase 3 — Dispatch work to subscribed devices** that run workers on-demand until the queue is drained

The current single-region `generate_summary` flow remains available but the new pipeline becomes the primary orchestration mode.

**Future architectural direction**: Move from a single fetcher-be process (one IP, N tokio workers) to a **device-subscription model** where multiple `fetcher-be` instances (each on its own machine/IP/proxy) **register themselves as devices** with the orch. When the orch has work, it:
1. Looks up subscribed devices
2. Probes each device's health
3. Asks healthy devices to allocate N workers on themselves
4. Monitors per-device progress; on 429, places that device on cooldown while others continue

The current plan must lay groundwork for this without implementing it. Specifically, the orch's Phase 3 becomes the **dispatcher** that will later select among devices, but in v1 it talks only to the current single-device fetcher.

---

## Current Architecture (What Changes)

**Today** (`orch/crates/app/src/app.rs:266-398`): `generate_summary(region_id)` does everything inline for ONE region: LLM queries -> store -> batch -> enqueue -> ensure workers -> return.

**Today's worker topology**: A single `fetcher-be` process (docker container `cortexmap-be`, see `docker-compose.app.yml:18-37`) runs all workers as `tokio::spawn` tasks (`fetcher-be/crates/cortexmap-be/src/worker_manager.rs:56-72`). All workers share one IP.

**After v1 (this plan)**:
- Phase 1, 2, 3 are distinct operations that can run independently on all regions
- Phase 3 **dispatches work via a new abstraction** that in v1 forwards to the existing single fetcher-be, but is shaped so subscribed-device routing can slot in later
- The orch stores a (currently one-row) list of "devices" with health status, enabling a future where many devices subscribe

**After v2 (next iteration, NOT in this plan)**:
- Multiple `fetcher-be` containers each subscribe to the orch with `POST /devices/subscribe { device_id, callback_url, capacity }`
- Each fetcher-be has a different IP/proxy
- Orch's dispatcher picks a healthy, non-cooling-down device, calls its `POST /workers/allocate`, monitors its queue contribution
- On 429 from any worker, the device self-reports cooldown to the orch; orch marks it unavailable

---

## Key Files Impacted

**orch/ (changed in this plan):**
| Layer | File | Change |
|-------|------|--------|
| Domain | `crates/domain/src/lib.rs`, new `pipeline_types.rs`, new `device_types.rs` | Pipeline types, device types |
| Infra trait | `crates/services/src/infra.rs` | New bulk-query and device methods |
| Infra impl | `crates/infra/src/pg.rs` | Diesel queries |
| Service | `crates/services/src/region_management.rs` | Bulk query generation |
| Service | `crates/services/src/batch_orchestration.rs` | Bulk enqueue, device dispatcher |
| Service | `crates/services/src/completion_watcher.rs` | Continuous fetch monitoring |
| App | `crates/app/src/services.rs`, `crates/app/src/app.rs` | New trait methods, pipeline orchestration |
| API/Server | `crates/api/src/orch_api.rs`, `crates/server/src/server.rs` | New endpoints |
| Migrations | `migrations/` | `devices` table, config defaults |

**fetcher-be/ (forward-looking, NOT changed in this plan except optional self-registration):**
| File | Future change |
|------|---------------|
| `crates/cortexmap-be/src/main.rs` or similar startup | Self-register with orch on boot (optional in v1) |
| `crates/cortexmap-fetcher/src/worker.rs` | Report 429 to device cooldown mechanism |
| `migrations/` | `fetch_tasks.device_id` column |

---

## Implementation Plan

### Step 1: Domain Types and Config

- [ ] **1.1 Add pipeline phase types in new `orch/crates/domain/src/pipeline_types.rs`**
  Define:
  ```
  PipelinePhase { Idle, GeneratingQueries, EnqueueingFetches, DispatchingFetch }
  QueryGenerationProgress { total_regions, completed, failed, skipped }
  BulkEnqueueProgress { total_queries, regions_processed, tasks_created, failed_queries }
  ContinuousFetchStatus { queue_depth, active_devices, total_active_workers, is_running }
  PipelineStatus { phase, phase_progress (serde_json::Value), started_at, updated_at }
  ```

- [ ] **1.2 Add device types in new `orch/crates/domain/src/device_types.rs`**
  Define:
  ```
  DeviceStatus { Healthy, Unhealthy, CoolingDown, Unreachable }
  Device {
      id: Uuid,
      name: String,                    // human-readable
      callback_url: String,            // HTTP base URL orch uses to call this device
      max_workers: u32,                // declared capacity
      status: DeviceStatus,
      last_heartbeat_at: Option<DateTime<Utc>>,
      cooldown_until: Option<DateTime<Utc>>,
      created_at: DateTime<Utc>,
  }
  DeviceSubscriptionRequest { name, callback_url, max_workers }
  DeviceHeartbeatRequest { device_id }
  ```
  All derive `Serialize, Deserialize, Clone, Debug`.
  
  **Rationale**: Matches the user's model — devices subscribe and are looked up when work arrives. In v1, there will typically be one device (the existing single fetcher-be), possibly auto-registered on boot or manually inserted via migration.

- [ ] **1.3 Add config keys to `ConfigKey` enum in `orch/crates/domain/src/lib.rs`**
  - `BulkQueryConcurrency` (default: `5`) — concurrent LLM calls in Phase 1
  - `BulkEnqueueConcurrency` (default: `10`) — concurrent fetcher enqueue calls in Phase 2
  - `ContinuousFetchPollSecs` (default: `30`) — Phase 3 poll interval
  - `DeviceHealthCheckTimeoutSecs` (default: `5`) — timeout when probing a device's `/health`
  - `DeviceHeartbeatStaleSecs` (default: `120`) — device is stale if no heartbeat in this window
  - `DeviceDefaultWorkerCount` (default: `2`) — workers to request per allocation call

- [ ] **1.4 Create migration: `devices` table and config defaults**
  New migration in `orch/migrations/`:
  ```sql
  CREATE TABLE devices (
      id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
      name TEXT NOT NULL UNIQUE,
      callback_url TEXT NOT NULL,
      max_workers INTEGER NOT NULL DEFAULT 2,
      status TEXT NOT NULL DEFAULT 'healthy'
          CHECK (status IN ('healthy', 'unhealthy', 'cooling_down', 'unreachable')),
      last_heartbeat_at TIMESTAMP,
      cooldown_until TIMESTAMP,
      created_at TIMESTAMP NOT NULL DEFAULT NOW(),
      updated_at TIMESTAMP NOT NULL DEFAULT NOW()
  );
  CREATE INDEX idx_devices_status ON devices(status) WHERE status = 'healthy';
  ```
  Plus `INSERT` statements for the new config keys (1.3).
  
  **Bootstrap row (optional)**: Insert one row corresponding to the existing single fetcher-be, using its `FETCHER_HTTP_ADDR` environment value. This preserves current behavior — Phase 3 dispatches to this one device.
  
  **Alternative bootstrap**: Leave the table empty and have `fetcher-be` self-register on startup (future work). For v1, prefer the migration-inserted bootstrap row to avoid requiring fetcher-be changes.

### Step 2: Infrastructure Layer — New DB Queries

- [ ] **2.1 Add `get_regions_without_queries` to `RegionMappingQueries` trait (`orch/crates/services/src/infra.rs`)**
  `async fn get_regions_without_queries(&self, database_url: &str) -> Result<Vec<RegionMapping>, Self::Error>` — returns regions with zero rows in `region_queries`.

- [ ] **2.2 Implement `get_regions_without_queries` in `orch/crates/infra/src/pg.rs`**
  Diesel query: `LEFT JOIN region_queries ... WHERE region_queries.id IS NULL`.

- [ ] **2.3 Add `get_unenqueued_queries` to `BatchManagement` trait**
  `async fn get_unenqueued_queries(&self, database_url: &str) -> Result<Vec<(Uuid /*query_id*/, Uuid /*region_id*/, String /*query_text*/)>, Self::Error>` — returns queries for regions with no active/completed batch.

- [ ] **2.4 Implement `get_unenqueued_queries` in `orch/crates/infra/src/pg.rs`**
  `NOT EXISTS` subquery pattern on `region_processing_batches` with status IN ('collecting', 'ready', 'processing', 'completed').

- [ ] **2.5 Add `get_pending_fetch_task_count` to `BatchManagement` trait**
  `async fn get_pending_fetch_task_count(&self, database_url: &str) -> Result<i64, Self::Error>` — counts `fetch_tasks WHERE status IN ('pending', 'in_progress')`.

- [ ] **2.6 Implement `get_pending_fetch_task_count` in `orch/crates/infra/src/pg.rs`**
  Simple Diesel count.

- [ ] **2.7 Add `DeviceRegistry` trait in `orch/crates/services/src/infra.rs`**
  New trait for device CRUD:
  ```rust
  #[async_trait]
  pub trait DeviceRegistry: Send + Sync {
      type Error: std::error::Error + Send + Sync + 'static;
      async fn register_device(&self, database_url: &str, req: DeviceSubscriptionRequest) -> Result<Device, Self::Error>;
      async fn list_healthy_devices(&self, database_url: &str) -> Result<Vec<Device>, Self::Error>;
      async fn list_all_devices(&self, database_url: &str) -> Result<Vec<Device>, Self::Error>;
      async fn update_device_status(&self, database_url: &str, device_id: Uuid, status: DeviceStatus, cooldown_until: Option<DateTime<Utc>>) -> Result<(), Self::Error>;
      async fn update_device_heartbeat(&self, database_url: &str, device_id: Uuid) -> Result<(), Self::Error>;
      async fn get_device(&self, database_url: &str, device_id: Uuid) -> Result<Option<Device>, Self::Error>;
      async fn delete_device(&self, database_url: &str, device_id: Uuid) -> Result<(), Self::Error>;
  }
  ```
  Add to the `Infra` super-trait alongside existing sub-traits.

- [ ] **2.8 Implement `DeviceRegistry` in `orch/crates/infra/src/pg.rs`**
  Standard Diesel INSERT/SELECT/UPDATE/DELETE against the `devices` table from 1.4.

### Step 3: Service Layer — Phase Implementations and Device Dispatcher

- [ ] **3.1 Add Phase 1 method to `RegionManagement` trait (`orch/crates/app/src/services.rs`)**
  `async fn generate_queries_for_all_regions(&self) -> Result<QueryGenerationProgress, Self::Error>;`

- [ ] **3.2 Implement Phase 1 in `orch/crates/services/src/region_management.rs`**
  In `OrchRegionManagement::generate_queries_for_all_regions`:
  1. `infra.get_regions_without_queries()` -> work-list
  2. Read `BulkQueryConcurrency` (default 5) and `QueryGenerationLimit` (default 3) from config
  3. `futures::stream::iter(regions).map(|r| async {...}).buffer_unordered(concurrency).collect::<Vec<_>>()`
  4. Each task calls existing `generate_queries(&region.name, count)` -> `insert_queries(region.id, queries)`
  5. Aggregate into `QueryGenerationProgress`
  
  **Reuse**: Inner loop uses existing `region_management.rs:179-243` LLM call and `pg.rs:201-228` insertion — only the orchestration wrapper is new.

- [ ] **3.3 Add Phase 2 method to `BatchOrchestration` trait**
  `async fn enqueue_all_queries(&self) -> Result<BulkEnqueueProgress, Self::Error>;`

- [ ] **3.4 Implement Phase 2 in `orch/crates/services/src/batch_orchestration.rs`**
  In `OrchBatchOrchestration::enqueue_all_queries`:
  1. `infra.get_unenqueued_queries()` -> work-list
  2. Group by `region_id`
  3. For each region (bounded by `BulkEnqueueConcurrency`):
     a. `create_batch(region_id, query_count)`
     b. Sequentially: `enqueue_fetch_task(query, region_id, Priority::Background)` for each query
     c. `add_tasks_to_batch(batch_id, all_task_ids)`
     d. On empty `task_ids`: `update_batch_status(batch_id, Failed, "No papers found")` — existing pattern (`app.rs:374-385`)
  4. Aggregate into `BulkEnqueueProgress`
  
  **Reuse**: All inner calls are existing methods. Fetcher's `/enqueue` already does ESearch + task creation.

- [ ] **3.5 Introduce `DeviceDispatcher` abstraction in a new file `orch/crates/services/src/device_dispatcher.rs`**
  The key architectural piece for subscription-based routing. Definition:
  ```rust
  #[async_trait]
  pub trait DeviceDispatcher {
      type Error;
      /// Probe a device's /health endpoint, returns Ok(true) if healthy
      async fn probe_device(&self, device: &Device) -> Result<bool, Self::Error>;
      /// Ask a device to allocate N workers on itself
      async fn allocate_workers_on_device(&self, device: &Device, count: u32, retry_config: &FetcherRetryConfig) -> Result<Vec<String>, Self::Error>;
      /// Query a device for its current worker status
      async fn get_worker_status_on_device(&self, device: &Device) -> Result<Vec<WorkerStatus>, Self::Error>;
      /// Stop workers on a device
      async fn stop_workers_on_device(&self, device: &Device, worker_ids: &[String]) -> Result<u32, Self::Error>;
  }
  ```
  Implementation `HttpDeviceDispatcher` uses `HttpClient` infra trait to call:
  - `GET {device.callback_url}/fetcher-be/health` — probe
  - `POST {device.callback_url}/fetcher-be/api/queue/workers/allocate` — allocate
  - `GET {device.callback_url}/fetcher-be/api/queue/workers/status` — status
  - `POST {device.callback_url}/fetcher-be/api/queue/workers/stop` — stop
  
  **These are exactly the same HTTP endpoints fetcher-be already exposes today**. The dispatcher just parameterizes the base URL by device. In v1 with a single device, this behaves identically to the current direct-call model.

- [ ] **3.6 Refactor existing `ensure_workers_allocated` to use `DeviceDispatcher`**
  Current `batch_orchestration.rs:341-460` hardcodes the fetcher URL from env var / config. Refactor it so:
  - It loads healthy devices via `DeviceRegistry::list_healthy_devices()`
  - For each healthy device, probes via `DeviceDispatcher::probe_device()`
  - For devices that pass probe, calls `DeviceDispatcher::allocate_workers_on_device()` with `DeviceDefaultWorkerCount`
  - Updates device health status based on probe results
  
  **Backward compatibility**: If `devices` table has exactly one row pointing at the current fetcher URL (the bootstrap row from 1.4), behavior is unchanged.

- [ ] **3.7 Add Phase 3 method to `CompletionOrchestrator` trait**
  `async fn run_continuous_fetch(&self) -> Result<ContinuousFetchStatus, Self::Error>;`

- [ ] **3.8 Implement Phase 3 in `orch/crates/services/src/completion_watcher.rs`**
  `run_continuous_fetch`:
  1. Call refactored `ensure_workers_allocated()` (from 3.6) — this now probes and allocates across all healthy devices
  2. Read `ContinuousFetchPollSecs` (default 30)
  3. Loop:
     a. `get_pending_fetch_task_count()` — if 0, exit loop
     b. `DeviceRegistry::list_healthy_devices()` — get current device list
     c. For each device: probe via `DeviceDispatcher::probe_device()`. Update device status based on result (healthy/unhealthy/unreachable).
     d. For each healthy device: query worker count via `DeviceDispatcher::get_worker_status_on_device()`. If 0 workers, re-allocate on that device.
     e. Sleep `ContinuousFetchPollSecs`
  4. Return `ContinuousFetchStatus`
  
  **Key design property**: The loop is device-agnostic. Adding more devices later (via self-registration) doesn't require changing Phase 3 logic — it will automatically include new devices in probe and allocation.

### Step 4: Application Layer — Pipeline Orchestration

- [ ] **4.1 Add pipeline methods to `OrchApp` (`orch/crates/app/src/app.rs`)**
  - `generate_all_queries(&self) -> Result<QueryGenerationProgress, E>`
  - `enqueue_all_fetches(&self) -> Result<BulkEnqueueProgress, E>`
  - `start_continuous_fetch(&self) -> Result<(), E>` — spawns `services.run_continuous_fetch()` as `tokio::spawn`

- [ ] **4.2 Add device management methods to `OrchApp`**
  - `register_device(&self, req: DeviceSubscriptionRequest) -> Result<Device, E>` — calls `infra.register_device()`
  - `list_devices(&self) -> Result<Vec<Device>, E>`
  - `device_heartbeat(&self, device_id: Uuid) -> Result<(), E>`
  - `deregister_device(&self, device_id: Uuid) -> Result<(), E>`

- [ ] **4.3 Add pipeline status tracking**
  Add `pipeline_status: Arc<RwLock<PipelineStatus>>` field to `OrchApp` (or `OrchServer`). Each phase method updates the status. Expose via `get_pipeline_status()`.

- [ ] **4.4 (Optional) `run_full_pipeline` convenience method**
  Chains Phase 1 -> Phase 2 -> Phase 3 as a single background task, updating status at each transition.

### Step 5: API Layer — New Endpoints

- [ ] **5.1 Add methods to `OrchApi` trait (`orch/crates/api/src/lib.rs`)**
  Pipeline:
  - `generate_all_queries`, `enqueue_all_fetches`, `start_continuous_fetch`, `get_pipeline_status`
  Devices:
  - `register_device`, `list_devices`, `device_heartbeat`, `deregister_device`

- [ ] **5.2 Implement in `orch/crates/api/src/orch_api.rs`**
  Standard delegation + error mapping pattern (same as existing methods like `orch_api.rs:49-57`).

- [ ] **5.3 Add HTTP handlers and routes (`orch/crates/server/src/server.rs`)**
  
  Pipeline routes under `/orch/api/pipeline/`:
  | Method | Path | Behavior |
  |--------|------|----------|
  | `POST` | `/pipeline/generate-queries` | Spawns Phase 1 background, returns `{ "started": true }` |
  | `POST` | `/pipeline/enqueue-fetches` | Spawns Phase 2 background |
  | `POST` | `/pipeline/start-fetch` | Spawns Phase 3 background |
  | `POST` | `/pipeline/run` | Spawns full pipeline background |
  | `GET` | `/pipeline/status` | Returns `PipelineStatus` snapshot |
  
  Device routes under `/orch/api/devices/`:
  | Method | Path | Behavior |
  |--------|------|----------|
  | `POST` | `/devices/subscribe` | Body: `DeviceSubscriptionRequest`, creates/updates device row |
  | `GET` | `/devices` | List all devices with status |
  | `POST` | `/devices/{id}/heartbeat` | Update `last_heartbeat_at` |
  | `DELETE` | `/devices/{id}` | Remove device |

### Step 6: Refactor Existing `generate_summary`

- [ ] **6.1 Make `generate_summary` reuse pre-generated queries**
  In `orch/crates/app/src/app.rs:generate_summary` (around line 295-308), check stored queries before calling LLM:
  ```
  let existing = self.services.get_queries(region_id).await?;
  let queries = if !existing.is_empty() {
      existing.iter().map(|q| q.query_text.clone()).collect()
  } else {
      self.services.generate_queries(&region_name, query_count).await?
  };
  ```

### Step 7: Forward-Looking Groundwork (Deferred Features)

These prepare for the full device-aware retry but are NOT implemented in this iteration.

- [ ] **7.1 Document the device self-registration contract**
  Add a `docs/` markdown or code comments specifying: "A fetcher-be instance self-registers by calling `POST /orch/api/devices/subscribe { name, callback_url, max_workers }` on startup. It sends `POST /devices/{id}/heartbeat` every 30 seconds. If no heartbeat arrives within `DeviceHeartbeatStaleSecs`, the device is marked `Unhealthy`." This is documentation-only in v1 — fetcher-be doesn't need to implement it yet. *Exception to the "don't create docs" rule: this is an API contract doc needed for the subscription protocol.*

- [ ] **7.2 Stub the cooldown-reporting endpoint**
  Add (but don't yet consume) `POST /orch/api/devices/{id}/cooldown { until_ts }` that sets `cooldown_until` and status to `CoolingDown`. Phase 3 already respects `DeviceStatus::CoolingDown` in `list_healthy_devices()` (which filters to `status = 'healthy'`). This lets a future fetcher-be self-report rate-limiting without another orch release.

- [ ] **7.3 Leave `device_id` hooks in domain types**
  Add optional `device_id: Option<Uuid>` to `AllocateWorkersRequest` and `WorkerStatus` in `orch/crates/domain/src/worker_types.rs`. Not used in v1 but available for future per-device routing inside a multi-device fetcher-be deployment.

---

## Verification Criteria

- Phase 1 generates queries for all regions lacking them; idempotent
- Phase 2 creates batches and enqueues fetch tasks for all unenqueued queries; idempotent
- Phase 3 probes devices, allocates workers on healthy ones, monitors queue, exits when empty
- `devices` table has one bootstrap row by default matching the current single fetcher-be
- All new device endpoints (`subscribe`, `list`, `heartbeat`, `delete`) work end-to-end
- `DeviceDispatcher` in v1 with one device behaves identically to the current direct-call pattern
- Existing `generate_summary` single-region flow continues to work and reuses pre-generated queries
- Existing `CompletionWatcher` background loop is unaffected
- Pipeline status queryable via `GET /pipeline/status`

---

## Potential Risks and Mitigations

1. **LLM cost explosion in Phase 1**
   1000 regions x 3 queries each = many LLM calls.
   *Mitigation*: `BulkQueryConcurrency` (default 5) + idempotency. Add `region_filter` or `limit` params later if needed.

2. **NCBI rate limiting in Phase 2**
   *Mitigation*: `BulkEnqueueConcurrency` (default 10). Fetcher already retries on 429 (`retry.rs:29-38`). Device subscription model will fully address this in v2.

3. **DB contention during bulk operations**
   *Mitigation*: Bounded concurrency. `FOR UPDATE SKIP LOCKED` already handles worker contention (`std-infra/src/task_queue.rs:106-137`).

4. **Long-running HTTP requests**
   *Mitigation*: All phase endpoints spawn background tasks and return immediately; progress via `GET /pipeline/status`.

5. **`generate_summary` conflicts with bulk batches**
   *Mitigation*: Existing active-batch guard (`app.rs:270-283`) already handles this correctly.

6. **Device registry empty on fresh deployment**
   If someone deploys without running the bootstrap migration, Phase 3 finds no devices and does nothing.
   *Mitigation*: Migration 1.4 inserts a bootstrap row. Also: fall back to env var `FETCHER_HTTP_ADDR` if `devices` table is empty (existing code path in `batch_orchestration.rs:346-363`). Log a warning when this fallback triggers.

7. **Stale devices in registry**
   Devices that were registered but are now offline pollute the healthy list.
   *Mitigation*: `DeviceHeartbeatStaleSecs` (default 120s). Phase 3's probe step updates device status based on actual reachability. A periodic "reap" sweep can mark stale devices `Unhealthy`.

8. **Race: device marked healthy mid-cooldown**
   If a device self-reports cooldown but Phase 3's health probe succeeds during the cooldown window, it might re-allocate workers prematurely.
   *Mitigation*: `list_healthy_devices()` must filter `cooldown_until IS NULL OR cooldown_until < NOW()`. Probe alone doesn't flip status to `Healthy` — the query respects both probe result and cooldown timestamp.

9. **Device callback_url unreachable from orch**
   E.g., devices behind NAT. In v1 this is fine (single-device, well-known URL). In v2 it may require reverse-tunnels or a polling model.
   *Mitigation*: Document the network requirement. Consider a future "poll-based" variant where devices long-poll the orch for work instead of orch push.

---

## Alternative Approaches

1. **Poll-based instead of push-based device model**: Devices long-poll orch for work instead of orch calling into devices. Better for NAT'd devices but more orch load. Not v1 — stick with push (matches current architecture).

2. **Skip device abstraction in v1**: Keep Phase 3 hardcoded to single fetcher URL, defer all device work to v2. *Drawback*: v2 would require refactoring the not-yet-existing Phase 3 code. *Benefit*: smaller v1 scope. **Decision**: Include the `DeviceRegistry` + `DeviceDispatcher` now because they're cheap and unlock v2 cleanly.

3. **Cron-based instead of API-triggered pipeline**: Run phases on a schedule. Could be a config option added later.

4. **Streaming progress via SSE**: Real-time pipeline progress. Nice-to-have, not essential.

5. **Cursor-based partial Phase 1**: `?limit=N&offset=M` for incremental LLM calls. Worth considering for cost control.

---

## Architecture Evolution

### v0 (current)
```
orch --HTTP--> fetcher-be (single process, N tokio workers, one IP)
                  |
                  +-- workers share single outbound IP -> NCBI
                  +-- rate limit on that IP stalls everything
```

### v1 (this plan)
```
orch
  |
  +-- devices table (1 bootstrap row -> fetcher-be callback_url)
  |
  +-- DeviceDispatcher --HTTP--> fetcher-be (unchanged)
         probe /health
         allocate /workers/allocate
         status   /workers/status
         stop     /workers/stop

Phase 3 loop:
  for device in list_healthy_devices():
      probe()
      ensure workers allocated
      monitor queue
```

### v2 (future, unblocked by this plan)
```
orch
  |
  +-- devices table (many rows, each = a fetcher-be instance)
  |
  +-- DeviceDispatcher
         |
         +--> fetcher-be #1 (IP A)
         +--> fetcher-be #2 (IP B, proxy)
         +--> fetcher-be #3 (IP C, different region)

Subscription flow:
  fetcher-be boot -> POST /orch/api/devices/subscribe { callback_url, max_workers }
  periodic       -> POST /orch/api/devices/{id}/heartbeat
  on 429         -> POST /orch/api/devices/{id}/cooldown { until_ts }

Phase 3 loop (unchanged from v1):
  for device in list_healthy_devices():   # cooldown_until filter excludes rate-limited devices
      probe()
      allocate as needed
  NCBI rate limits one device -> others keep working
```

**The v1->v2 transition requires NO orch code changes beyond wiring — only fetcher-be learning to self-subscribe and report cooldowns.**
