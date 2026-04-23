# Orchestration System Implementation Plan

**Created:** 2026-02-14  
**Status:** In Progress  
**Goal:** Build orch service to orchestrate paper fetching → LLM processing pipeline

---

## Phase 1: Clean Up & Prepare Fetcher ✅ COMPLETE

### 1.1 Rollback Incorrect Migration ✅
- [x] Revert migration `2026-02-14-000001-0000_add_llm_enqueued_at`
- [x] Create new migration `2026-02-14-000002-drop_region_summary_constraints`
  - [x] Drop `region_summary_name_key` unique constraint
  - [x] Drop `region_summary_region_id_key` unique constraint
  - [x] Keep everything else unchanged (no llm_enqueued_at)
- [x] Run migration on database
- [x] Verify `psql` shows correct schema

### 1.2 Add GET /tasks Endpoint to Fetcher ✅
- [x] Add `get_tasks_by_status()` to `cortexmap-infra/src/infra.rs` trait
  - Signature: `async fn get_tasks_by_status(&self, status: &str, limit: i32) -> Result<Vec<FetchTask>, InfraError>`
- [x] Implement in `std-infra/src/task_queue.rs`
  - Query: `SELECT * FROM fetch_tasks WHERE status = $1 ORDER BY completed_at ASC LIMIT $2`
- [x] Delegate in `std-infra/src/infra.rs`
- [x] Add handler in `cortexmap-be/src/server.rs`
  - `GET /api/queue/tasks?status=completed&limit=100`
- [x] Add route
- [x] Build and test
- [ ] Manual test: `curl http://localhost:8080/fetcher-be/api/queue/tasks?status=completed&limit=10` (needs server running)

**Deliverable:** Fetcher exposes clean API with no LLM concerns

---

## Phase 2: Orch Database Schema ✅ COMPLETE

### 2.1 Design Schema ✅
- [x] Create `orch/migrations/` directory
- [x] Create migration `2026-02-14-000001-initial_orch_schema`
- [x] Define `processed_fetch_tasks` table:
  ```sql
  CREATE TABLE processed_fetch_tasks (
    fetch_task_id BIGINT PRIMARY KEY,
    region_id UUID NOT NULL,
    pmc_id TEXT NOT NULL,
    processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    brainatlas_status TEXT NOT NULL DEFAULT 'pending',
    brainatlas_started_at TIMESTAMP,
    brainatlas_completed_at TIMESTAMP,
    error_message TEXT,
    CONSTRAINT brainatlas_status_check CHECK (brainatlas_status IN ('pending', 'in_progress', 'completed', 'failed'))
  );
  CREATE INDEX idx_processed_fetch_tasks_status ON processed_fetch_tasks(brainatlas_status);
  CREATE INDEX idx_processed_fetch_tasks_region ON processed_fetch_tasks(region_id);
  ```
- [x] Define `orch_config` table:
  ```sql
  CREATE TABLE orch_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
  );
  ```
- [x] Insert default config values:
  ```sql
  INSERT INTO orch_config (key, value, description) VALUES
    ('completion_poll_interval_secs', '30', 'How often to check for completed fetch tasks'),
    ('region_scan_interval_secs', '86400', 'How often to scan for stale region summaries (24h)'),
    ('max_parallel_process_calls', '10', 'Max concurrent calls to brainatlas /process'),
    ('summary_staleness_days', '30', 'Consider summaries older than N days stale'),
    ('fetcher_base_url', 'http://localhost:8080/fetcher-be', 'Fetcher service URL'),
    ('brainatlas_base_url', 'http://localhost:8081/brainatlas-be', 'Brainatlas service URL');
  ```
- [x] Write down.sql for rollback
- [x] Run migration
- [x] Verify with psql

### 2.2 Infra Layer (Redesigned with Clean Separation) ✅
**Architecture:** Followed brainatlas-be pattern - lazy DB initialization, modular structure

- [x] Define traits in `services/src/infra.rs`:
  - `EnvInfra` - environment variable access
  - `OrchDatabase` - all database operations
  - `Infra` - blanket impl combining both traits
- [x] Define domain types in `services/src/infra.rs`:
  - `NewProcessedFetchTask`
  - `ProcessedFetchTask`
  - `OrchConfig`
- [x] Create `infra/src/env.rs` with `OrchEnvInfra`
- [x] Create `infra/src/pg.rs` with `OrchPostgresql`:
  - Lazy pool initialization (follows brainatlas pattern)
  - Implements `OrchDatabase` trait
  - All queries use `interact()` for thread safety
- [x] Create `infra/src/models.rs` with Diesel models:
  - DB-specific models with Diesel derives
  - `From` conversions between DB and services types
- [x] Update `infra/src/infra.rs`:
  - `OrchInfra` holds `OrchEnvInfra` + `OrchPostgresql`
  - Delegates to specialized modules
  - Implements both `EnvInfra` and `OrchDatabase`
- [x] Update `infra/src/lib.rs` - exports (pg module kept private)
- [x] Add `chrono` to services dependencies
- [x] Build and verify - compiles cleanly

**Deliverable:** Orch has its own database tables

---

## Phase 3: Orch Service Skeleton

### 3.1 Create Directory Structure
```
orch/
├── Cargo.toml
├── migrations/
│   └── 2026-02-14-000001-initial_orch_schema/
├── crates/
│   ├── domain/        (domain types)
│   ├── rpc-types/     (proto generated code)
│   ├── infra/         (database trait)
│   ├── services/      (business logic)
│   ├── api/           (API trait)
│   ├── app/           (app layer)
│   └── server/        (HTTP server)
```

- [ ] Create workspace `orch/Cargo.toml`
- [ ] Create all crate directories
- [ ] Create `orch/diesel.toml`
- [ ] Create `orch/.env` with `DATABASE_URL`

### 3.2 Domain Crate
- [ ] Create `orch/crates/domain/Cargo.toml`
- [ ] Create `orch/crates/domain/src/lib.rs`
- [ ] Define domain types:
  - `ProcessedFetchTask`
  - `OrchConfig`
  - `RegionPipelineStatus` enum (matches proto)
  - `RegionSummary` (reuse from brainatlas or define here)

### 3.3 RPC Types (Proto Bindings)
- [ ] Copy `proto/orch/orch.proto` setup from earlier design
- [ ] Create `orch/crates/rpc-types/Cargo.toml`
- [ ] Create `orch/crates/rpc-types/build.rs`
  - Add serde derives for all types
- [ ] Create `orch/crates/rpc-types/src/lib.rs`
- [ ] Build and verify proto generation

### 3.4 Infra Crate (Database)
- [ ] Create `orch/crates/infra/Cargo.toml` (diesel, tokio, etc.)
- [ ] Create Diesel schema: `orch/crates/infra/src/schema.rs`
- [ ] Create models: `orch/crates/infra/src/models.rs`
  - `ProcessedFetchTaskRow`
  - `OrchConfigRow`
- [ ] Create infra trait: `orch/crates/infra/src/infra.rs`
  ```rust
  #[async_trait]
  pub trait OrchInfra {
    async fn get_processed_task(&self, fetch_task_id: i64) -> Result<Option<ProcessedFetchTask>>;
    async fn insert_processed_task(&self, task: NewProcessedTask) -> Result<()>;
    async fn update_brainatlas_status(&self, fetch_task_id: i64, status: &str) -> Result<()>;
    async fn get_config(&self, key: &str) -> Result<Option<String>>;
    async fn update_config(&self, key: &str, value: &str) -> Result<()>;
    async fn get_all_config(&self) -> Result<Vec<OrchConfig>>;
  }
  ```
- [ ] Create Postgres impl: `orch/crates/infra/src/pg.rs`
- [ ] Wire up in `orch/crates/infra/src/lib.rs`

### 3.5 Services Crate
- [ ] Create `orch/crates/services/Cargo.toml`
- [ ] Create completion watcher: `orch/crates/services/src/completion_watcher.rs`
  - Stub that logs "running completion watcher"
- [ ] Create config service: `orch/crates/services/src/config.rs`
  - CRUD operations for orch_config
- [ ] Create `orch/crates/services/src/lib.rs`

### 3.6 API Crate
- [ ] Create `orch/crates/api/Cargo.toml`
- [ ] Create API trait: `orch/crates/api/src/api.rs`
  ```rust
  #[async_trait]
  pub trait OrchApi {
    type Error;
    async fn search_region(&self, req: SearchRegionRequest) -> Result<SearchRegionResponse, Self::Error>;
    async fn get_region_status(&self, region_id: Uuid) -> Result<GetRegionStatusResponse, Self::Error>;
    async fn invalidate_region(&self, region_id: Uuid) -> Result<InvalidateRegionResponse, Self::Error>;
    async fn get_config(&self) -> Result<GetConfigResponse, Self::Error>;
    async fn update_config(&self, req: UpdateConfigRequest) -> Result<UpdateConfigResponse, Self::Error>;
    async fn get_pipeline_stats(&self) -> Result<GetPipelineStatsResponse, Self::Error>;
  }
  ```
- [ ] Create error types: `orch/crates/api/src/error.rs`
- [ ] Create impl: `orch/crates/api/src/orch_api.rs` (all stubs returning NotImplemented)

### 3.7 App Crate
- [ ] Create `orch/crates/app/Cargo.toml`
- [ ] Create services trait: `orch/crates/app/src/services.rs`
- [ ] Create app: `orch/crates/app/src/app.rs`
  - `OrchApp` struct with services field
- [ ] Wire in `orch/crates/app/src/lib.rs`

### 3.8 Server Crate
- [ ] Create `orch/crates/server/Cargo.toml`
- [ ] Create server: `orch/crates/server/src/server.rs`
  - Axum setup with all routes stubbed
- [ ] Create main: `orch/crates/server/src/main.rs`
  - Load env, create infra, create app, start server
- [ ] Add health check: `GET /orch/health`
- [ ] Build entire workspace: `cargo build`
- [ ] Run server: `cargo run -p server`
- [ ] Test: `curl http://localhost:8081/orch/health`

**Deliverable:** Orch service runs, all endpoints return stubs/501

---

## Phase 4: Brainatlas /process Implementation

### 4.1 Design Processing Pipeline
- [ ] Document required dependencies:
  - S3 client (reuse from fetcher or use aws-sdk-s3)
  - Text chunker (implement or use library)
  - Embedding API client (OpenAI/local model)
  - LLM API client (OpenAI/Anthropic)
- [ ] Design chunking strategy (chunk size, overlap)
- [ ] Design prompt template for summarization

### 4.2 Add S3 Download to Brainatlas Infra
- [ ] Add S3Infra trait to `brainatlas-be/crates/infra`
  - `async fn download_s3(&self, key: &str) -> Result<String>`
- [ ] Implement using aws-sdk-s3 or reuse fetcher's S3 client
- [ ] Add to app dependencies

### 4.3 Implement Chunking
- [ ] Create `brainatlas-be/crates/services/src/chunker.rs`
  - Function: `fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String>`
- [ ] Add tests for chunker
- [ ] Expose via services layer

### 4.4 Implement Embedding Client
- [ ] Create `brainatlas-be/crates/infra/src/embedding.rs`
  - Trait: `async fn generate_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>`
- [ ] Implement using OpenAI API or local model
- [ ] Add to infra context

### 4.5 Implement Vector DB Storage
- [ ] Choose vector DB (pgvector extension or external)
- [ ] If pgvector:
  - [ ] Migration: add vector extension
  - [ ] Migration: create embeddings table
  - [ ] Implement insert/query methods
- [ ] Add to infra

### 4.6 Implement LLM Summarization
- [ ] Create `brainatlas-be/crates/infra/src/llm.rs`
  - Trait: `async fn summarize(&self, chunks: Vec<String>) -> Result<String>`
- [ ] Implement using OpenAI/Anthropic API
- [ ] Design prompt: "Summarize the following brain region research..."
- [ ] Add to infra context

### 4.7 Wire Up /process Endpoint
- [ ] Update `brainatlas-be/crates/api/src/brainatlas_api.rs`
  - Implement `process_region()`:
    1. Download all S3 files
    2. Concatenate text
    3. Chunk text
    4. Generate embeddings → store in vector DB
    5. Generate summary via LLM
    6. Insert into `region_summary` table
    7. Return success
- [ ] Add error handling for each step
- [ ] Add logging
- [ ] Build and test manually

**Deliverable:** `POST /brainatlas-be/api/process` works end-to-end

---

## Phase 5: Implement Orch Completion Watcher

### 5.1 HTTP Client Setup
- [ ] Add reqwest to `orch/crates/services/Cargo.toml`
- [ ] Create client wrapper: `orch/crates/services/src/http_client.rs`
  - `async fn get_completed_tasks(fetcher_url, limit) -> Result<Vec<Task>>`
  - `async fn get_task_components(fetcher_url, task_id) -> Result<Components>`
  - `async fn process_region(brainatlas_url, req) -> Result<ProcessResponse>`

### 5.2 Implement Completion Watcher Logic
- [ ] Update `orch/crates/services/src/completion_watcher.rs`:
  ```rust
  pub async fn run_completion_watcher(ctx: Context) {
    loop {
      let interval = get_config("completion_poll_interval_secs");
      sleep(interval).await;
      
      // 1. Get completed tasks from fetcher
      let tasks = http_client.get_completed_tasks(100).await?;
      
      // 2. Filter already processed
      let new_tasks = filter_unprocessed(tasks, &ctx.infra).await?;
      
      // 3. Process each
      for task in new_tasks {
        process_single_task(task, &ctx).await?;
      }
    }
  }
  
  async fn process_single_task(task, ctx) {
    // Insert with status='pending'
    ctx.infra.insert_processed_task(...).await?;
    
    // Get components
    let components = http_client.get_task_components(task.id).await?;
    
    // Update status='in_progress'
    ctx.infra.update_brainatlas_status(task.id, "in_progress").await?;
    
    // Call brainatlas
    match http_client.process_region(region_id, components.s3_keys).await {
      Ok(_) => ctx.infra.update_brainatlas_status(task.id, "completed").await?,
      Err(e) => {
        ctx.infra.update_brainatlas_status(task.id, "failed").await?;
        log error
      }
    }
  }
  ```
- [ ] Add error handling and retry logic
- [ ] Add concurrency limit (use semaphore)

### 5.3 Wire into Server
- [ ] Spawn completion watcher in `orch/crates/server/src/main.rs`
  - `tokio::spawn(completion_watcher::run(ctx))`
- [ ] Add graceful shutdown
- [ ] Test end-to-end

**Deliverable:** Orch automatically discovers completed fetch tasks and triggers brainatlas processing

---

## Phase 6: Implement Orch User-Facing Endpoints

### 6.1 Search Region
- [ ] Implement `search_region()` in `orch/crates/api/src/orch_api.rs`:
  1. Query `region_mapping` by name/id
  2. Query `region_summary` by region_id
  3. If empty → enqueue fetch task → return status FETCH_QUEUED
  4. If exists → return summaries + status DONE
- [ ] Add fetcher enqueue client call
- [ ] Wire to HTTP handler
- [ ] Test

### 6.2 Get Region Status
- [ ] Implement `get_region_status()`:
  1. Check if fetch task exists (call fetcher API)
  2. Check if in processed_fetch_tasks
  3. Check if summaries exist
  4. Derive status enum (NOT_STARTED → FETCH_QUEUED → ... → DONE)
- [ ] Wire to handler
- [ ] Test

### 6.3 Invalidate Region
- [ ] Implement `invalidate_region()`:
  1. Mark existing summaries (add `invalidated_at` column or just accept multiple)
  2. Re-enqueue fetch task with high priority
  3. Remove from processed_fetch_tasks (so it gets re-processed)
- [ ] Wire to handler
- [ ] Test

### 6.4 Config Endpoints
- [ ] Implement `get_config()` - return all orch_config rows
- [ ] Implement `update_config()` - update specific key
- [ ] Wire to handlers
- [ ] Test

### 6.5 Pipeline Stats
- [ ] Implement `get_pipeline_stats()`:
  - Count by brainatlas_status
  - Recent processed tasks
  - Average processing time
- [ ] Wire to handler
- [ ] Test

**Deliverable:** All orch endpoints functional

---

## Phase 7: Testing & Integration

### 7.1 Manual End-to-End Test
- [ ] Start fetcher-be
- [ ] Start brainatlas-be
- [ ] Start orch
- [ ] POST /orch/api/regions/search with new region
- [ ] Verify fetcher enqueues task
- [ ] Verify fetcher worker downloads paper
- [ ] Verify orch completion watcher triggers brainatlas
- [ ] Verify summary appears in region_summary
- [ ] GET /orch/api/regions/{id}/status → returns DONE

### 7.2 Test Invalidate Flow
- [ ] POST /orch/api/regions/{id}/invalidate
- [ ] Verify re-fetch triggers
- [ ] Verify re-processing triggers
- [ ] Verify new summary row inserted

### 7.3 Test Config Updates
- [ ] PATCH /orch/api/config {completion_poll_interval_secs: 10}
- [ ] Verify watcher picks up new interval (check logs)

### 7.4 Error Handling Tests
- [ ] Test fetcher down → orch logs error, continues
- [ ] Test brainatlas down → marks status=failed
- [ ] Test invalid region_id → returns error

**Deliverable:** Full pipeline works reliably

---

## Phase 8: Documentation & Cleanup

- [ ] Update README with orch service info
- [ ] Document environment variables needed
- [ ] Document API endpoints (or generate OpenAPI spec)
- [ ] Add docker-compose entry for orch
- [ ] Clean up debug logs
- [ ] Final build check: `cargo build --release`

**Deliverable:** System ready for deployment

---

## Success Criteria

- ✅ User searches for region → gets summaries or queued status
- ✅ Pipeline runs automatically (fetcher → orch → brainatlas)
- ✅ Invalidation works (forces refresh)
- ✅ Config is adjustable without code changes
- ✅ All services are decoupled (no cross-DB dependencies)
- ✅ Graceful error handling throughout

---

## Notes

- Checkboxes track progress
- Each phase builds on previous
- Can parallelize Phase 4 (brainatlas) and Phase 3 (orch skeleton)
- Estimated total: 30-40 hours of implementation
