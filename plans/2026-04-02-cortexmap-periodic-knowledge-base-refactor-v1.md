# CortexMap: Periodic Knowledge Base Ingestion Refactor

## Objective

Restructure the CortexMap pipeline to separate **knowledge ingestion** (periodic, background) from **summary generation** (on-demand, LLM-only). Currently, the "Generate Summary" button triggers the entire pipeline: query generation, fetching, chunking, embedding, and summarization. The new architecture:

1. **Periodic ingestion** (configurable interval): For each brain region, orch generates queries, fetches papers via workers, chunks text, and stores embeddings in the vector DB. This runs automatically and continuously as a background knowledge base builder.
2. **On-demand summary generation** (`POST /generate`): Uses only the LLM + existing vector DB embeddings (RAG) to produce a summary. No fetching occurs. Fast and stateless with respect to the fetch pipeline.

---

## Current Architecture (Reference)

| Step | Trigger | Service | What Happens |
|---|---|---|---|
| 1 | User clicks "Generate" | orch | Generates 3 LLM queries via brainatlas-be |
| 2 | Same request | orch → fetcher-be | Enqueues fetch tasks (NCBI search → S3 upload) |
| 3 | Background watcher | orch | Polls batch until all fetch tasks complete |
| 4 | Background watcher | orch → brainatlas-be | Sends S3 keys to `/api/process` (chunk + embed + RAG summarize) |
| 5 | Frontend | brainatlas-fe | Polls batch status, shows progress |

**Problems with current approach:**
- Fetching and summarization are coupled — every summary request re-fetches papers
- The 60/60 bug: upsert on `(pmc_id, query)` returns already-completed tasks, making new batches appear instantly done
- No incremental knowledge building — same papers are re-fetched for the same region
- Summary generation is slow because it waits for the entire fetch+process pipeline

---

## Target Architecture

```
┌─────────────────────────────────────────────────────────┐
│                PERIODIC INGESTION (Background)           │
│                                                         │
│  orch (scheduler)                                       │
│    │                                                    │
│    ├─ For each region:                                  │
│    │   1. Generate N queries (brainatlas-be LLM)        │
│    │   2. Enqueue fetch tasks (fetcher-be)              │
│    │   3. Wait for completion (existing watcher)        │
│    │   4. Send S3 keys to chunk+embed (brainatlas-be)   │
│    │      (NO summary generation in this step)          │
│    │                                                    │
│    └─ Sleep for configurable interval, repeat           │
│                                                         │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│           ON-DEMAND SUMMARY (User Request)               │
│                                                         │
│  POST /orch/api/regions/{id}/generate                   │
│    │                                                    │
│    └─ Call brainatlas-be RAG summarize endpoint          │
│       (reads existing embeddings from vector DB)         │
│       → Returns summary immediately (seconds, not mins) │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

---

## Implementation Plan

### Phase 1: New Configuration Keys

- [x] **1.1** Add new `ConfigKey` variants to `orch/crates/domain/src/lib.rs`:
  - `IngestionIntervalSecs` — interval between full ingestion cycles (e.g., default `3600` = 1 hour)
  - `IngestionBatchSize` — how many regions to process per cycle (default: all regions, or a configurable subset to avoid overwhelming NCBI/S3)
  - `IngestionEnabled` — boolean flag to enable/disable the periodic ingestion (`true` by default)

  **Rationale:** The existing `ConfigKey` enum + `orch_config` DB table pattern is already established with runtime mutability via `PATCH /orch/api/config`. Adding keys here makes the new scheduler immediately configurable without code changes.

- [x] **1.2** Seed default values for the new keys in a new SQL migration under `orch/migrations/`. Follow the existing pattern in `2026-02-14-000001_create_orch_config/up.sql` which inserts default rows into `orch_config`.

  **Rationale:** Ensures the system works out of the box after migration without manual config insertion.

- [x] **1.3** Verify the existing `QueryGenerationLimit` config key (default `3`) is sufficient or needs adjustment. Currently at `orch/crates/domain/src/lib.rs:35`. This already controls "how many queries per region." No new key needed unless the periodic flow requires a different count than the on-demand flow.

  **Rationale:** Reuse existing config rather than duplicating.

---

### Phase 2: Periodic Ingestion Scheduler (Orch)

- [x] **2.1** Create a new service trait `IngestionScheduler` in `orch/crates/app/src/services.rs` (or extend `CompletionOrchestrator`). This trait should define:
  - `run_ingestion_cycle(&self) -> Result<IngestionCycleResult, Error>` — processes all (or a batch of) regions
  - `ingest_region(&self, region_id: Uuid) -> Result<RegionIngestionResult, Error>` — processes a single region

  **Rationale:** Clean separation of concerns. The ingestion scheduler is a distinct background concern from the existing completion watcher. However, it may reuse the same polling/processing machinery.

- [x] **2.2** Create the concrete implementation in a new file `orch/crates/services/src/ingestion_scheduler.rs`. The ingestion cycle logic:
  1. Read `IngestionEnabled` config — if `false`, skip cycle
  2. Load all regions from `region_mapping` (reuse `get_all_regions`)
  3. For each region (respecting `IngestionBatchSize`):
     a. Check if region already has an active batch (`Collecting`/`Ready`/`Processing`) — skip if so
     b. Generate queries via brainatlas-be LLM (reuse `generate_queries`)
     c. Store queries in `region_queries` (reuse `store_queries`)
     d. Create a new batch with status `Collecting`
     e. Enqueue fetch tasks in fetcher-be (reuse `enqueue_fetch_task`)
     f. Store task IDs in batch

  **Rationale:** This is essentially the same logic as the current `generate_summary` in `orch/crates/app/src/app.rs:244-337`, but triggered by a scheduler instead of a user request, and without the summary generation step at the end.

- [x] **2.3** Spawn the ingestion scheduler loop in `orch/crates/app/src/app.rs` `init()` method, alongside the existing completion watcher spawn at `app.rs:25-69`. Pattern:
  ```
  loop {
      if ingestion_enabled {
          run_ingestion_cycle().await;
      }
      sleep(ingestion_interval_secs).await;
  }
  ```

  **Rationale:** Follows the same pattern as the existing completion watcher loop. Both run concurrently as independent background tasks.

- [x] **2.4** Fix the existing task upsert bug that causes stale completed tasks to appear in new batches. In `fetcher-be/crates/std-infra/src/task_queue.rs`, the `ON CONFLICT (pmc_id, query) DO UPDATE` clause must:
  - Reset `status` to `'pending'`
  - Clear `stream_message_id` to `NULL`
  - Reset `completed_at` to `NULL`
  - Reset `worker_id` to `NULL`

  This applies to both `StdTaskQueue::enqueue_task` (around line 60-70) and `RedisTaskQueue::enqueue_task` (around line 907). Additionally, remove the early-return guard at `task_queue.rs:937-940` that checks `stream_message_id.is_some()`, so re-enqueued tasks are properly added back to the Redis stream.

  **Rationale:** This is the root cause of the 60/60 bug. Without this fix, the periodic ingestion would have the same problem — re-enqueuing the same PMC IDs would return stale completed task IDs. The ingestion scheduler will repeatedly enqueue the same regions, so this fix is a hard prerequisite.

---

### Phase 3: Modify the Completion Watcher (Chunk + Embed Only)

- [x] **3.1** Modify the `process_batch` method in `orch/crates/services/src/completion_watcher.rs:244-484` to perform **chunking and embedding only**, without triggering summary generation. This means:
  - The call to `brainatlas-be/api/process` at `completion_watcher.rs:414-429` should be replaced with a call to a new endpoint (see Phase 4) that only chunks and stores embeddings
  - Remove or skip the summary generation step from the `/api/process` pipeline

  **Rationale:** The completion watcher's job becomes: "when a batch's fetch tasks are done, chunk the text and store embeddings in the vector DB." Summary generation is decoupled to the user-triggered route.

- [x] **3.2** After successful chunking+embedding, mark the batch as `Completed` (or a new status like `Indexed` if we want to distinguish "embeddings stored" from "summary generated"). The batch no longer represents the full pipeline end-to-end — it represents knowledge ingestion completion.

  **Rationale:** The batch lifecycle simplifies. It tracks: `Collecting → Ready → Processing (chunking/embedding) → Completed`. Summary generation is a separate, stateless operation.

---

### Phase 4: New brainatlas-be Endpoint for Chunk+Embed Only

- [x] **4.1** Create a new endpoint in brainatlas-be: `POST /brainatlas-be/api/ingest` (or `/api/chunk-and-embed`). This endpoint performs Steps 1-5 of the current `/api/process` pipeline in `brainatlas-be/crates/app/src/app.rs:53-187`:
  1. Download S3 content
  2. Chunk text (1000 chars, 200 overlap)
  3. Content deduplication via SHA-256
  4. Generate embeddings in parallel
  5. Store embeddings in `brain_region_embeddings` with full source metadata

  But it does **NOT** perform Step 6 (RAG summarization loop) or Step 7 (update summary text).

  **Rationale:** Separating ingestion from summarization at the API level. The existing `/api/process` endpoint can be preserved for backward compatibility or refactored.

- [x] **4.2** Decide whether the new endpoint should create a `region_summary` placeholder row (currently done at `app.rs:174-187`). Options:
  - **Option A:** Don't create a summary row during ingestion. The summary row is only created when the user requests a summary. Embeddings are stored without a `summary_id` FK, or with a nullable FK.
  - **Option B:** Create a placeholder summary row with `summary = NULL` as currently done, and the generate route fills it in later.

  Recommendation: **Option A** is cleaner. Embeddings should reference the region directly (they already have `region_id`), not a summary that doesn't exist yet. The `summary_id` FK on `brain_region_embeddings` becomes optional/nullable or is dropped in favor of `region_id` as the primary grouping key.

  **Rationale:** The current design ties embeddings to a specific summary, but in the new model, embeddings are a shared knowledge base queried by any summary generation request.

- [x] **4.3** If Option A is chosen, add a migration to make `summary_id` nullable on `brain_region_embeddings`, or add an index on `region_id` to support efficient vector search without going through `summary_id`.

  **Rationale:** The vector search query at `brainatlas-be/crates/infra/src/vectordb.rs:186-198` already filters by `region_id`, not `summary_id`. So the search itself works without changes — only the FK constraint and insert logic need adjustment.

---

### Phase 5: Refactor the Generate Summary Route (LLM-Only)

- [x] **5.1** Refactor `POST /orch/api/regions/{id}/generate` in `orch/crates/app/src/app.rs` `generate_summary` method. The new behavior:
  1. Check if the region has any embeddings in the vector DB (via a new count query or by checking `brain_region_embeddings WHERE region_id = ?`)
  2. If no embeddings exist, return an appropriate error/message (e.g., "Knowledge base not yet built for this region. Ingestion is in progress.")
  3. If embeddings exist, call brainatlas-be's summarization endpoint (see 5.2) directly
  4. Return the summary to the user

  **Rationale:** This makes the generate route fast and stateless. No batch creation, no fetch task enqueuing, no polling. The user gets a summary in seconds, not minutes.

- [x] **5.2** Create a new endpoint in brainatlas-be: `POST /brainatlas-be/api/summarize` (or reuse/modify the existing `/api/process` with a flag). This endpoint performs **only** the RAG summarization loop from `brainatlas-be/crates/app/src/app.rs:209-360`:
  1. Load system/user prompts
  2. Define the `search_embeddings` tool
  3. Run the multi-turn tool-calling loop (max 5 iterations)
  4. The `search_embeddings` tool queries `brain_region_embeddings` filtered by `region_id`
  5. Return the final summary text

  Request body: `{ region_id, region_name, chat_model?, embedding_model? }`
  Response body: `{ summary_id, summary_text }`

  **Rationale:** This endpoint is the pure "brain" — it reads from the vector DB and writes a summary. No S3 interaction, no chunking.

- [x] **5.3** Store the generated summary in `region_summary` table with the `content_hash` of the embeddings that were used, and `batch_id` of the most recent completed ingestion batch (for traceability).

  **Rationale:** Preserves the existing summary storage model. The `content_hash` dedup still works — if the same embeddings are used, the same hash is produced, and we can skip re-summarization.

- [x] **5.4** Update the orch's `generate_summary` response to be synchronous or near-synchronous. Since no batch/polling is needed for summarization, the response can include the summary text directly, or a very short-lived async operation. Options:
  - **Option A (Sync):** The orch blocks on the brainatlas-be `/api/summarize` call and returns the summary in the same HTTP response. Typical RAG summarization takes 10-30 seconds.
  - **Option B (Async with fast poll):** Create a lightweight "summary job" that the frontend polls. Simpler frontend changes but more complex backend.

  Recommendation: **Option A** — synchronous. The RAG loop takes ~10-30s which is acceptable for a user-triggered action. The frontend can show a spinner during the request. This eliminates the entire batch/polling machinery for summaries.

  **Rationale:** Dramatic simplification. No more batch status polling, no more cookies for batch IDs, no more complex state machine for summary generation.

---

### Phase 6: Frontend Updates

- [x] **6.1** Simplify `brainatlas-fe/src/components/RegionDetail.jsx`. Remove:
  - Batch ID cookie management (`RegionDetail.jsx:21-69`)
  - Batch status polling loop (`RegionDetail.jsx:112-122`)
  - The `batchId` / `batchStatus` state variables (`RegionDetail.jsx:75-76`)
  - The multi-status progress display (fetching, queued, processing indicators)

  **Rationale:** With synchronous summary generation, the frontend just needs: a "Generate" button that makes a POST, shows a spinner, and displays the result.

- [x] **6.2** Update the "Generate Summary" button handler to:
  1. Set `isGenerating = true`
  2. `POST /orch/api/regions/{id}/generate`
  3. On success: update the summaries list with the new summary
  4. On failure: show error
  5. Set `isGenerating = false`

  **Rationale:** Simple request-response pattern replaces the complex polling-based pipeline tracking.

- [x] **6.3** Add a new UI element showing the **ingestion status** for a region. This is separate from summary generation:
  - "Knowledge base: 240 papers indexed" (count of embeddings/chunks)
  - "Last ingestion: 2 hours ago"
  - "Next ingestion: in 58 minutes"
  - Optionally: "Ingestion in progress..." if there's an active batch for this region

  **Rationale:** Users should see the health of the knowledge base independently from summaries.

- [x] **6.4** Retain the region status polling (`RegionDetail.jsx:97-107`) but simplify the status enum. The pipeline status can be reduced to:
  - `NoData` — no embeddings exist for this region yet
  - `Ingesting` — periodic ingestion is currently running for this region
  - `Ready` — knowledge base has data, user can generate summaries
  - `Generating` — a summary is currently being generated (frontend-only state)

  **Rationale:** Simpler mental model for users. The 8-status pipeline (`NotStarted`, `FetchQueued`, `Fetching`, `FetchFailed`, `LlmQueued`, `Processing`, `Done`, `Invalidated`) becomes unnecessary when ingestion and summarization are decoupled.

---

### Phase 7: API Route & orch Cleanup

- [x] **7.1** Deprecate or remove the `get_batch_status` route (`GET /orch/api/batches/{id}/status`) and the `get_active_batch` route (`GET /orch/api/regions/{id}/active-batch`). These were needed for the old polling-based pipeline. In the new model:
  - Ingestion status can be a new route: `GET /orch/api/regions/{id}/ingestion-status`
  - Summary generation is synchronous, so no polling endpoint is needed

  **Rationale:** Simplification. Remove dead code paths.

- [x] **7.2** Add a new route: `GET /orch/api/regions/{id}/knowledge-status` that returns:
  - Total embeddings count for the region
  - Last ingestion timestamp
  - Current ingestion batch status (if any)
  - Next scheduled ingestion time

  **Rationale:** Replaces the old batch status with a knowledge-base-centric view.

- [x] **7.3** Update `get_region_status` in `orch/crates/app/src/app.rs:84-118` to reflect the new status model. Instead of mapping batch statuses, it should check: does the region have embeddings? Is ingestion running? This simplifies the logic significantly.

  **Rationale:** Aligns the API with the new decoupled architecture.

---

### Phase 8: Incremental Ingestion (Optimization)

- [x] **8.1** Implement incremental ingestion logic in the ingestion scheduler. When re-processing a region:
  - Generate queries (may produce the same queries if the region name hasn't changed)
  - For queries that already exist in `region_queries`, skip regeneration
  - The fetcher upsert (after Phase 2.4 fix) will properly re-fetch papers, but we can also check if content has changed via S3 ETags or content hashing before re-embedding

  **Rationale:** Avoids unnecessary work on repeated ingestion cycles. NCBI results change slowly — re-embedding identical content wastes compute and API calls (embedding generation costs money).

- [x] **8.2** Add a `last_ingested_at` timestamp column to `region_mapping` (or a new `region_ingestion_status` table) to track when each region was last processed. The scheduler can use this to prioritize regions that haven't been ingested recently.

  **Rationale:** Enables fair scheduling across hundreds of brain regions. Without this, the scheduler would always start from region #1 every cycle.

- [x] **8.3** Consider adding a content hash per-region per-ingestion to detect when new papers are available. If the NCBI search returns the same PMC IDs and the S3 content hasn't changed, skip the chunk+embed step entirely.

  **Rationale:** The existing `content_hash` on `region_summary` already implements this pattern for summaries. Extending it to embeddings avoids redundant embedding API calls.

---

## Verification Criteria

- The periodic ingestion scheduler runs on a configurable interval and processes all brain regions
- The `IngestionIntervalSecs`, `IngestionBatchSize`, and `IngestionEnabled` config keys are runtime-configurable via `PATCH /orch/api/config`
- The fetcher upsert bug is fixed: re-enqueuing the same `(pmc_id, query)` creates a genuinely pending task
- After ingestion completes, `brain_region_embeddings` contains chunks with full source metadata for the region
- `POST /orch/api/regions/{id}/generate` returns a summary without triggering any fetch operations
- Summary generation completes in ~10-30 seconds (RAG loop only)
- The frontend shows a simple generate button + spinner, with no batch polling
- The frontend displays knowledge base status (embedding count, last ingestion time)
- Existing summaries are preserved and accessible after the refactor
- The system handles the cold-start case: if no embeddings exist for a region, the generate route returns an informative error

---

## Potential Risks and Mitigations

1. **NCBI Rate Limiting During Bulk Ingestion**
   Mitigation: The `IngestionBatchSize` config limits how many regions are processed per cycle. The existing fetcher worker count and `fetcher_empty_queue_sleep_secs` already throttle NCBI requests. Additionally, consider adding a per-cycle delay between regions.

2. **Embedding API Costs**
   Mitigation: Content deduplication via SHA-256 hash (already exists in `brainatlas-be/crates/app/src/app.rs:113-124`) prevents re-embedding identical content. The incremental ingestion (Phase 8) further reduces redundant calls.

3. **Database Growth from Periodic Embeddings**
   Mitigation: Implement a retention policy — delete embeddings older than N ingestion cycles, or keep only the latest set per region. The `summary_id` FK or a new `ingestion_batch_id` column can track which ingestion produced each embedding.

4. **Breaking the Frontend During Transition**
   Mitigation: Implement backend changes first (Phases 1-5) while keeping the old generate route working. Then update the frontend (Phase 6). The old batch-based flow can coexist with the new flow during the transition.

5. **Cold Start: No Embeddings When User First Generates**
   Mitigation: The generate route should check for embeddings and return a clear message. Optionally, provide a "Force Ingest" button that triggers immediate ingestion for a specific region (essentially the old manual flow, but only for bootstrapping).

6. **Long-Running Synchronous Summary Requests**
   Mitigation: The RAG loop is bounded to 5 iterations (`brainatlas-be/crates/app/src/app.rs:246`). Set an HTTP timeout of 60s on the orch→brainatlas call. The frontend should handle timeout errors gracefully.

---

## Alternative Approaches

1. **Keep Batch-Based Summary Generation (Async):** Instead of making generate synchronous, keep the batch/polling pattern but only for the RAG step. This preserves the current frontend architecture but adds complexity. Trade-off: more complex but handles very long summarization times better.

2. **Event-Driven Instead of Polling Scheduler:** Use Redis pub/sub or a cron job instead of a sleep-loop scheduler. Trade-off: more operationally complex but more precise timing and better observability.

3. **Merge Orch and Brainatlas-be:** Since the orch already delegates everything to brainatlas-be for LLM work, merging them would reduce HTTP hops. Trade-off: simpler deployment but violates the current separation of concerns and makes the monolith harder to scale independently.

4. **SSE/WebSocket for Summary Generation:** Instead of synchronous HTTP, use server-sent events to stream the summary as it's generated. Trade-off: better UX (user sees progress) but requires more frontend/backend work.
