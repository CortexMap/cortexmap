# Orch Pipeline Decoupling — v4 (Post-Implementation Gap Closure)

## Status Snapshot

v1 of the decoupled pipeline is **implemented and compiles cleanly**. This plan (v4) supersedes v3 and addresses the remaining gaps between what shipped and what the user's original three-part request prescribed, plus lays minimal groundwork for the device-subscription follow-up.

v3 is archived conceptually — its device-registry sections are deferred wholesale to a separate follow-up plan and should not be implemented in this iteration.

## Objective

Close the functional gaps in the current pipeline implementation so that it truly satisfies the user's original three requirements:

1. Generate queries for all regions and store them ✅ (done)
2. Use stored queries to fetch IDs for all queries via the existing queue ✅ (done)
3. **Run fetcher all the time until queue is not empty** ⚠️ (partially done — only checks once per 1-hour cycle)

Secondarily, add cheap forward-compatibility for the upcoming device-subscription retry model so v2 does not require a breaking API change.

## What's Already Done (do not re-touch)

- Domain config key `ConfigKey::PipelineCycleSleepSecs` at `orch/crates/domain/src/lib.rs:56`
- Migration `orch/migrations/2026-04-18-000001-add_pipeline_config/up.sql` seeds `pipeline_cycle_sleep_secs = 3600`
- Trait `PipelineRunner` at `orch/crates/app/src/services.rs:200-217`
- Infra queries at `orch/crates/infra/src/pg.rs:782-870` (`get_regions_without_queries`, `get_all_regions_with_queries`, `get_pending_fetch_task_count`)
- Service impl `OrchPipelineRunner` at `orch/crates/services/src/pipeline_runner.rs`
- Wiring through `OrchServices` at `orch/crates/services/src/services.rs:25, 35, 351-375`
- Background loop in `OrchApp::init()` at `orch/crates/app/src/app.rs:77-139`
- `generate_summary` short-circuits to pre-generated queries at `orch/crates/app/src/app.rs:352-386`

## Implementation Plan

### Step 1 — Make Phase 3 Actually Continuous (Core Gap)

The current loop sleeps `pipeline_cycle_sleep_secs` (1 hour) between everything, so worker death mid-cycle leaves the queue idle for up to an hour. The user explicitly asked for "run fetcher all the time until queue is not empty." This requires decoupling the Phase 3 cadence from the Phase 1/2 cadence.

- [x] Task 1.1. Add a new config key `ConfigKey::FetcherMonitorIntervalSecs` in `orch/crates/domain/src/lib.rs` with default 30 seconds. Rationale: Phase 3's fast-path cadence must be independent of the 1-hour query-discovery cadence.

- [x] Task 1.2. Add migration `orch/migrations/<next-timestamp>-add_fetcher_monitor_interval/up.sql` inserting `fetcher_monitor_interval_secs = '30'` into `orch_config`, with matching `down.sql`. Rationale: Ship with a sensible default without requiring operator action.

- [x] Task 1.3. Restructure Phase 3 in `OrchPipelineRunner::ensure_fetcher_running` (`orch/crates/services/src/pipeline_runner.rs:333-451`) to become a bounded monitor-until-drained loop: while `get_pending_fetch_task_count() > 0`, re-probe worker status, re-allocate if any workers have died, sleep `fetcher_monitor_interval_secs`, repeat. Exit when queue is empty OR a max-inner-iterations safety cap is hit (to avoid starving Phase 1/2). Rationale: Directly satisfies requirement #3. The safety cap prevents a bug or permanently-failing queue from blocking the outer cycle indefinitely.

- [x] Task 1.4. Alternative (simpler) shape: keep Phase 3 as a one-shot "ensure allocated" call, but spawn a **second independent background task** in `OrchApp::init()` that runs the fast-cadence monitor loop (30s) independently of the slow pipeline cycle (1h). The fast loop only calls `get_pending_fetch_task_count` + `ensure_workers_allocated` when count > 0. Rationale: Cleaner separation of concerns — slow-cycle task handles query/paper discovery, fast-cycle task handles keep-alive. This is the **recommended shape**.

- [x] Task 1.5. Wire the chosen shape into `OrchApp::init()` at `orch/crates/app/src/app.rs:77-139`. If going with Task 1.4, add a third `tokio::spawn` block for the monitor loop. Rationale: Keeps `OrchApp::init` as the single source of truth for background task lifecycle.

### Step 2 — Consume `get_pending_fetch_task_count` (Remove Dead API)

The trait method is fully wired end-to-end but never called from the app layer. Step 1 above will consume it, but until then it's dead code.

- [x] Task 2.1. Confirm the call site added in Step 1 references `pipeline_services.get_pending_fetch_task_count().await` and that the return value gates the monitor loop. Rationale: Ensures the method earns its keep.

### Step 3 — Unhardcode Phase 2 Values

Phase 2 currently hardcodes `page_size: 20` and `max_retry_attempts: 3` at `orch/crates/services/src/pipeline_runner.rs:223-224`, while Phase 3 correctly reads from config at lines `414-420`. Unify.

- [x] Task 3.1. Read `max_retry_attempts` in Phase 2 from `ConfigKey::FetcherMaxRetryAttempts` (already exists), using the same pattern as Phase 3's `orch/crates/services/src/pipeline_runner.rs:414-420`. Default to 3 if not present. Rationale: Configuration consistency — a single knob controls retry behavior across both enqueue and worker allocation paths.

- [x] Task 3.2. Add a new `ConfigKey::EnqueuePageSize` with default 20, migrate it, and read it in Phase 2 in place of the hardcoded `20`. Rationale: Page size is a natural tuning knob for polite NCBI API usage; hardcoding it prevents rate-limit tuning without a redeploy.

### Step 4 — Observability (Minimal)

The current implementation logs per phase but offers no HTTP endpoint to inspect pipeline health. A single read-only status endpoint is low-cost and high-value for debugging.

- [x] Task 4.1. Add a lightweight `GET /orch/api/pipeline/status` endpoint that returns `{ regions_without_queries: usize, regions_with_queries: usize, pending_fetch_tasks: i64, worker_count: usize }` by calling existing infra queries + `get_worker_status`. No shared state or progress tracking needed. Rationale: Enables ops/UI to see whether the pipeline is healthy without tailing logs. Avoids the heavyweight `PipelineStatus` shared-state design from v3.

- [x] Task 4.2. Wire the handler in `orch-server` alongside the existing endpoints. Rationale: Single-file touchpoint.

### Step 5 — Forward-Compat Groundwork for Device-Subscription Retry

The user's stated next iteration is device-based retry (one worker per device, IP-level cooldowns). These are the **minimum** additive changes that keep the wire protocol forward-compatible so the follow-up does not require a breaking orch release.

- [x] Task 5.1. Add optional `device_id: Option<String>` field to `AllocateWorkersRequest` in `orch/crates/domain/src/worker_types.rs`. Orch will always send `None` in v1. Fetcher ignores unknown/None. Rationale: When a future fetcher-be instance allocates workers on behalf of a specific device, orch can populate this field without any wire-format change.

- [x] Task 5.2. Add optional `device_cooldown_secs: Option<u64>` field to the `FetcherRetryConfig` sub-struct (if it exists in worker types; otherwise document as a TODO in `worker_types.rs`). Rationale: Once fetcher-be starts reporting 429-driven cooldowns, orch needs to transmit the cooldown duration. Adding the optional field now avoids a v2 API break.

- [x] Task 5.3. Document (inline `//` comments) that these fields are reserved for the device-subscription follow-up and should remain `None`/unset in v1. Rationale: Prevents a future contributor from "cleaning up" the unused fields.

### Step 6 — Acceptance Test

- [x] Task 6.1. Run the project end-to-end (`cargo check --workspace` in `orch/` already passes). Manually verify in a dev environment: (a) starting orch with an empty `region_queries` table causes Phase 1 to populate it, (b) Phase 2 creates batches for regions with queries, (c) killing workers while the queue has pending tasks results in workers being re-allocated within `fetcher_monitor_interval_secs` rather than `pipeline_cycle_sleep_secs`. Rationale: Verifies the core gap from the current implementation is closed.

## Verification Criteria

- Phase 3 re-allocates dead workers within `fetcher_monitor_interval_secs` (default 30s), not `pipeline_cycle_sleep_secs` (default 3600s).
- `get_pending_fetch_task_count` is called on every fast-monitor iteration; when it returns 0 the monitor sleeps without probing workers.
- Phase 2 reads `max_retry_attempts` and page size from config rather than hardcoded literals.
- `GET /orch/api/pipeline/status` returns a JSON body reflecting current counts (observable via `curl`).
- `AllocateWorkersRequest` serialises/deserialises without field with `device_id = None` (backwards compatible with existing fetcher-be).
- `cargo check --workspace` passes with zero new warnings.
- Existing single-region `generate_summary` flow continues to work unchanged (regression check).

## Potential Risks and Mitigations

1. **Two concurrent background loops could race on worker allocation**
   Mitigation: `ensure_workers_allocated` is already idempotent — it checks active worker count before allocating. The fast and slow loops both calling it is safe. If desired, guard with an `Arc<Mutex<()>>` to serialise.

2. **Fast monitor loop could hammer the fetcher `/workers/status` endpoint**
   Mitigation: Default 30s cadence is gentle. Endpoint is an in-memory lookup on the fetcher side. Make the interval configurable for tuning.

3. **`ensure_workers_allocated` failure spams logs at 30s cadence when fetcher is down**
   Mitigation: Use `tracing::warn!` (already done), and consider exponential backoff inside the monitor loop on consecutive failures (defer to a follow-up unless it becomes noisy in practice).

4. **Safety cap on Step 1.3 inner loop could strand tasks if pipeline cycle sleep is long**
   Mitigation: Use Task 1.4 shape (two independent background tasks) instead — sidesteps the issue entirely.

5. **Adding new config keys without migrations leaves prod DBs without defaults**
   Mitigation: Every new `ConfigKey` variant gets a matching migration in the same commit; the code reads `unwrap_or(<sensible default>)` so missing rows don't panic.

6. **Forward-compat fields on `AllocateWorkersRequest` get serialized even when `None`**
   Mitigation: Use `#[serde(skip_serializing_if = "Option::is_none")]` on the new fields so the v1 wire format is identical to today.

## Alternative Approaches

1. **Single loop with variable sleep** — Keep one background task but choose sleep duration based on queue state: sleep 30s if queue non-empty, sleep 3600s if empty. Simpler but couples phase cadences. Not recommended.

2. **Event-driven instead of polling** — Have fetcher-be push a webhook to orch on worker death or queue-drain. More efficient but introduces fetcher→orch coupling that doesn't exist today. Defer to a larger architectural refactor.

3. **Merge Phase 3 into Phase 2** — Only ensure workers after creating a batch. Simpler code but violates requirement #3 (workers should stay alive for pre-existing tasks too, e.g., after an orch restart). Not recommended.

4. **Move monitor loop into fetcher-be itself** — Have fetcher-be self-heal dead workers without orch involvement. Cleaner but expands fetcher-be scope; orch loses visibility into worker lifecycle. Reserve for when device-subscription is implemented (at which point the orch *must* orchestrate across devices anyway).

## Out of Scope (Deferred to Follow-Up Plans)

- Device subscription registry (`devices` table, `DeviceRegistry` trait, `/devices/*` endpoints)
- `DeviceDispatcher` abstraction
- Per-device IP cooldown tracking
- Fetcher-be self-registration on boot
- Device health-probe flow
- Per-worker `device_id` tagging in `fetch_tasks` table

These are all part of the **next plan** (device-subscription retry model). This plan only lays minimal forward-compat groundwork (Step 5).
