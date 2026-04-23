# CortexMap — Orchestrator Architecture Plan

## 1. Overview

The system processes brain region data from academic papers stored in S3, generates LLM summaries, and serves them to clients. As of this plan, three backend services exist:

- **fetcher-be** — downloads papers from PubMed, stores raw content in S3, tracks tasks in PostgreSQL
- **brainatlas-be** — processes S3 files (chunk → embed → summarize), stores summaries in PostgreSQL
- **orch** *(new)* — the single public-facing API; orchestrates the full pipeline between fetcher and brainatlas

Clients never talk to fetcher or brainatlas directly. All requests go through orch.

---

## 2. Services and Their Responsibilities

### 2.1 fetcher-be (unchanged)

- Accepts enqueue requests from orch via `POST /api/queue/enqueue`
- Downloads paper content (PDF, abstract, summary) from PubMed/PMC to S3
- Tracks state in `fetch_tasks` and `fetch_task_components`
- Has its own worker pool for download jobs
- Does not know about brainatlas or orch

### 2.2 brainatlas-be (simplified)

- Accepts processing requests from orch via `POST /process`
- Given S3 file references from a completed fetch task:
  - Chunks the content
  - Embeds chunks into the vector DB
  - Generates an LLM summary
  - Appends a new row to `region_summary` with a timestamp
- Does not poll or self-schedule — orch drives it
- Does not know about fetcher or orch

### 2.3 orch *(new service in this repo)*

- The **only** public-facing API
- Owns the coordination state via the `fetch_tasks.llm_enqueued_at` column
- Runs two background loops
- Exposes config, search, status, invalidation, and stats endpoints
- All interval values and priority mappings come from a DB config table — nothing hardcoded

---

## 3. Database Schema Changes

### 3.1 `fetch_tasks` — one new column

```sql
ALTER TABLE fetch_tasks
  ADD COLUMN llm_enqueued_at TIMESTAMP DEFAULT NULL;
```

This is the sole coordination primitive between orch's two background loops and its API handlers.

- `NULL` = fetch is complete but brainatlas has not yet been called
- `NOT NULL` = brainatlas has been called for this task

Invalidation resets this column to `NULL`, causing the completion watcher to re-hand the task off.

### 3.2 `region_summary` — drop the UNIQUE constraint

The current schema has `UNIQUE(region_id)`, allowing only one summary per region. This must be dropped to support multiple time-stamped summaries:

```sql
ALTER TABLE region_summary DROP CONSTRAINT region_summary_name_key;
```

Summaries are append-only. Nothing is ever deleted. Clients receive all summaries ordered by `created_at DESC`.

### 3.3 `orch_config` — new table

```sql
CREATE TABLE orch_config (
  key         TEXT PRIMARY KEY,
  value       TEXT NOT NULL,
  description TEXT,
  updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

Seed data (default values):

| key | default | description |
|---|---|---|
| `region_scan_interval_secs` | `86400` | How often the scanner checks all regions |
| `completion_poll_interval_secs` | `30` | How often the completion watcher polls |
| `summary_staleness_days` | `30` | Age at which a summary is considered stale |
| `default_priority` | `5` | Priority for scanner-triggered fetches |
| `user_requested_priority` | `8` | Priority when a search miss triggers a fetch |
| `invalidation_priority` | `10` | Priority for forced refresh |
| `max_parallel_brainatlas_calls` | `10` | Concurrency cap for brainatlas calls in watcher |

---

## 4. Orch — API Endpoints

Defined in `proto/orch/orch.proto`.

### `POST /api/regions/search`

The primary client endpoint. Replaces direct calls to brainatlas.

**Request**: `{ "region_id": "<uuid>" }`

**Behavior**:
1. Read all rows from `region_summary` for the given `region_id`, ordered `created_at DESC`
2. Derive the current `RegionPipelineStatus` (see §5)
3. If summaries exist → return them with `status = DONE`
4. If status is already in-flight → return empty summaries with the current status
5. If `NOT_STARTED` → enqueue via fetcher at `USER_REQUESTED` priority, return `status = FETCH_QUEUED`

**Response**:
```json
{
  "status": "DONE",
  "summaries": [
    { "summary": "...", "created_at": "2026-02-14T..." },
    { "summary": "...", "created_at": "2026-01-10T..." }
  ]
}
```

Client shows the most recent summary immediately. Older entries are available for history/comparison.

---

### `GET /api/regions/{id}/status`

Poll endpoint for in-flight pipelines.

**Response**: `RegionPipelineStatus` + `last_fetch_at`, `last_summary_at`, `summary_count`, `current_priority`

---

### `POST /api/regions/{id}/invalidate`

Forces a fresh fetch + process cycle without deleting existing summaries.

**Behavior**:
1. Find the most recent `fetch_task` for this region
2. If it exists and was recently completed: reset `llm_enqueued_at = NULL`, bump `priority = INVALIDATION` (from config)
3. If it is very stale or doesn't exist: create a new fetch task at `INVALIDATION` priority
4. Returns what action was taken in the `detail` field

Old summaries remain readable during the new cycle. A new row is appended to `region_summary` when the cycle completes.

---

### `GET /api/pipeline/stats`

Returns counts of regions in each `RegionPipelineStatus` state. Intended for dashboards and monitoring.

---

### `GET /api/config` / `PATCH /api/config`

Read and partially update the `orch_config` table. Only keys provided in the PATCH body are changed. Changes take effect on the next loop iteration without a restart.

---

## 5. Pipeline State Derivation

Orch derives `RegionPipelineStatus` by querying two tables. No service stores this enum explicitly.

| Condition | Status |
|---|---|
| No `fetch_task` row | `NOT_STARTED` |
| `fetch_task.status = pending` | `FETCH_QUEUED` |
| `fetch_task.status = in_progress` | `FETCHING` |
| `fetch_task.status = failed` | `FETCH_FAILED` |
| `fetch_task.status = completed` AND `llm_enqueued_at IS NULL` | `LLM_QUEUED` |
| `fetch_task.status = completed` AND `llm_enqueued_at IS NOT NULL` AND no `region_summary` row | `PROCESSING` |
| `region_summary` row exists AND `llm_enqueued_at IS NOT NULL` (stable) | `DONE` |
| `region_summary` row exists AND `llm_enqueued_at IS NULL` (reset by invalidate) | `INVALIDATED` |

---

## 6. Orch — Background Loops

Both loops read their interval from `orch_config` on each tick, so config changes apply without a restart.

### 6.1 Region Scanner

**Interval**: `region_scan_interval_secs` (default 24h)

```
for each region in region_mapping:
  latest_summary = most recent region_summary row
  if latest_summary is NULL
    OR (NOW() - latest_summary.created_at) > staleness_days:
      if no active fetch_task for this region:
        POST fetcher /api/queue/enqueue at NORMAL priority
```

Runs as a `tokio::spawn` loop. Entire scan is parallelised with `tokio::join_all` across regions, capped at `max_parallel_brainatlas_calls` (same config key, reused).

### 6.2 Completion Watcher

**Interval**: `completion_poll_interval_secs` (default 30s)

```
rows = SELECT * FROM fetch_tasks
       WHERE status = 'completed'
       AND llm_enqueued_at IS NULL

for each row (parallel, capped):
  POST brainatlas /process { s3_keys: [...from fetch_task_components...] }
  on 200: UPDATE fetch_tasks SET llm_enqueued_at = NOW() WHERE id = row.id
  on error: log, leave llm_enqueued_at NULL (will retry next tick)
```

The `llm_enqueued_at IS NULL` filter is idempotent — orch can crash and restart without re-processing already-handed-off tasks. Failed brainatlas calls are automatically retried on the next watcher tick.

---

## 7. Priority System

Named priority levels, mapped to integers stored in `fetch_tasks.priority`:

| Name | Integer | When used |
|---|---|---|
| `BACKGROUND` | 0 | Reserved, not used currently |
| `NORMAL` | 5 | Region scanner scheduled fetch |
| `USER_REQUESTED` | 8 | Search miss triggered the fetch |
| `INVALIDATION` | 10 | User called `/invalidate` |

Integers are read from `orch_config` at runtime, not hardcoded. Fetcher's worker pool already orders jobs by `priority DESC` — no fetcher changes needed.

---

## 8. Request Flow Diagrams

### Happy path — summary exists

```
Client → POST /api/regions/search { region_id }
  orch: SELECT region_summary WHERE region_id = ? ORDER BY created_at DESC
  → rows found
  orch: return { status: DONE, summaries: [...] }
```

### Search miss — nothing in DB

```
Client → POST /api/regions/search { region_id }
  orch: SELECT region_summary → empty
  orch: SELECT fetch_tasks → no active task
  orch → POST fetcher /api/queue/enqueue { region_id, priority: USER_REQUESTED }
  orch: return { status: FETCH_QUEUED, summaries: [] }

Client polls GET /api/regions/{id}/status every N seconds

  [fetcher worker]
    downloads paper → S3 → marks fetch_task completed

  [completion watcher tick]
    finds fetch_task with llm_enqueued_at IS NULL
    → POST brainatlas /process { s3_keys }
    → UPDATE fetch_tasks SET llm_enqueued_at = NOW()

  [brainatlas worker]
    chunks → embeds → summarizes
    → INSERT INTO region_summary ...

Client polls → status: DONE
Client → POST /api/regions/search → returns summary
```

### Invalidation

```
Client → POST /api/regions/{id}/invalidate
  orch: find latest completed fetch_task for region
  orch: UPDATE fetch_tasks SET llm_enqueued_at = NULL, priority = INVALIDATION
  orch: return { status: INVALIDATED, detail: "re-queued existing fetch task" }

[next completion watcher tick]
  finds the row (llm_enqueued_at IS NULL again)
  → POST brainatlas /process
  → UPDATE llm_enqueued_at = NOW()

[brainatlas]
  generates new summary → INSERT INTO region_summary (new row, old rows untouched)

Client → POST /api/regions/search
  → returns all summaries, newest first
```

---

## 9. Orch — Crate Structure

Follows the same layered architecture as `brainatlas-be`:

```
orch/
├── Cargo.toml              (workspace)
├── Dockerfile
├── migrations/
│   ├── 0001_add_llm_enqueued_at/
│   └── 0002_create_orch_config/
└── crates/
    ├── domain/             types: RegionPipelineStatus, Priority, ConfigEntry, RegionSummary
    ├── rpc-types/          prost-generated from proto/orch/orch.proto
    ├── infra/              Diesel: fetch_tasks, region_summary, orch_config queries
    ├── api/                trait BrainAtlasOrch + impl
    ├── services/           background loop logic (scanner, watcher)
    ├── app/                composes api + services
    └── server/             Axum HTTP server
```

---

## 10. What Is Not Changing

- Fetcher's worker pool, task schema (except `llm_enqueued_at`), and download logic are untouched
- Brainatlas keeps its existing `/process` endpoint; it does not need workers or a scheduler
- The `region_summary` table is append-only — no deletions, ever
- Redis is out of scope for this phase; can be added later as a cache layer in front of orch's search reads

---

## 11. Implementation Order

1. **DB migrations** — `llm_enqueued_at` column + `orch_config` table + drop `region_summary` unique constraint
2. **`orch/crates/domain`** — types matching the proto
3. **`orch/crates/infra`** — Diesel queries for all three tables
4. **`orch/crates/services`** — region scanner loop + completion watcher loop
5. **`orch/crates/api`** — trait + impl composing infra + service calls to fetcher/brainatlas
6. **`orch/crates/server`** — Axum handlers wired to the api trait
7. **Integration test** — full search miss → fetch → process → done flow against the live DB
