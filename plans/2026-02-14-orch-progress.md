# Orch Implementation Progress

**Last Updated:** 2026-02-14

---

## ✅ Phase 1: Clean Up & Prepare Fetcher - COMPLETE

### 1.1 Rollback Incorrect Migration ✅
- ✅ Reverted `2026-02-14-000001-0000_add_llm_enqueued_at` migration  
- ✅ Created new migration: `2026-02-14-000002-drop_region_summary_constraints`
  - Dropped `region_summary_name_key` unique constraint
  - Dropped `region_summary_region_id_key` unique constraint
- ✅ Applied migration successfully
- ✅ Verified with psql

### 1.2 Add GET /tasks Endpoint to Fetcher ✅
- ✅ Added `get_tasks_by_status()` to `TaskQueueInfra` trait
- ✅ Implemented in `StdTaskQueue` with Diesel query
- ✅ Wired through `StdInfra` delegator
- ✅ Added HTTP handler `get_tasks_handler()`
- ✅ Added route `GET /fetcher-be/api/queue/tasks?status=completed&limit=100`
- ✅ Built successfully

**Result:** Fetcher has clean API with no LLM concerns, returns list of completed fetch tasks.

---

## ✅ Phase 2: Orch Database Schema - COMPLETE

### 2.1 Create Migration ✅
- ✅ Created `orch/migrations/2026-02-14-000001-initial_orch_schema/`
- ✅ Created `processed_fetch_tasks` table:
  - `fetch_task_id BIGINT PRIMARY KEY`
  - `region_id UUID NOT NULL`
  - `pmc_id TEXT NOT NULL`
  - `processed_at TIMESTAMP DEFAULT NOW()`
  - `brainatlas_status TEXT NOT NULL`
  - `brainatlas_started_at TIMESTAMP`
  - `brainatlas_completed_at TIMESTAMP`
  - `error_message TEXT`
  - Indexes on `brainatlas_status` and `region_id`
- ✅ Created `orch_config` table:
  - `key TEXT PRIMARY KEY`
  - `value TEXT NOT NULL`
  - `description TEXT`
  - `updated_at TIMESTAMP DEFAULT NOW()`
- ✅ Inserted default config values:
  - `completion_poll_interval_secs` = 30
  - `region_scan_interval_secs` = 86400 (24h)
  - `max_parallel_process_calls` = 10
  - `summary_staleness_days` = 30
  - `fetcher_base_url` = http://localhost:8080/fetcher-be
  - `brainatlas_base_url` = http://localhost:8081/brainatlas-be
- ✅ Applied migration successfully
- ✅ Verified with psql

### 2.2 Infra Layer (Redesigned with Separation) ✅
**Architecture Decision:** Followed brainatlas-be pattern with lazy DB initialization

- ✅ Defined traits in `services/src/infra.rs`:
  - `EnvInfra` - environment variable access
  - `OrchDatabase` - all database operations
  - `Infra` - blanket impl combining both
- ✅ Defined domain types in `services/src/infra.rs`:
  - `NewProcessedFetchTask`
  - `ProcessedFetchTask`
  - `OrchConfig`
- ✅ Created `infra/src/pg.rs` with `OrchPostgresql`:
  - Lazy pool initialization (follows brainatlas pattern)
  - Implements `OrchDatabase` trait
  - All queries use `interact()` for thread safety
- ✅ Created `infra/src/models.rs` with Diesel models:
  - DB-specific models with Diesel derives
  - `From` conversions between DB and services types
- ✅ Created `infra/src/env.rs` with `OrchEnvInfra`
- ✅ Updated `infra/src/infra.rs`:
  - `OrchInfra` holds `OrchEnvInfra` + `OrchPostgresql`
  - Delegates to specialized modules
  - No direct database logic
- ✅ Added `chrono` to services dependencies
- ✅ Built successfully with no errors

**Result:** Clean separation of concerns - env, database, and coordination logic are modular.

---

## 📊 Summary

| Component | Status | Notes |
|---|---|---|
| **Fetcher-be** | ✅ Ready | Clean API, no LLM coupling |
| **Brainatlas-be** | ⏳ Partial | `/process` endpoint stubbed |
| **Orch Database** | ✅ Complete | Schema + infra layer done |
| **Orch Service** | ⏳ Skeleton | Crates exist, needs implementation |

---

## 🎯 Next Steps

From `plans/2026-02-14-orch-implementation-v1.md`:

### Phase 3: Orch Service Skeleton (Partially Done)
- ✅ Crate structure exists
- ✅ Database infra complete
- ⏳ HTTP clients (fetcher, brainatlas)
- ⏳ Background loops (completion watcher, region scanner)

### Phase 4: Brainatlas /process Implementation
The heavy lifting - S3 → chunk → embed → summarize → DB

### Phase 5: Completion Watcher
Core orchestration: poll fetcher → call brainatlas → track in DB

### Phase 6: User-Facing Endpoints
Search, status, invalidate, config management
