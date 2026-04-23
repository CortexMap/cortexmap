# Remaining Implementation Tasks

**Date:** 2026-02-14  
**Status:** Infrastructure 100% Complete, Business Logic Pending

---

## ✅ What's Complete

### Database Infrastructure (100%)
- ✅ All migrations applied across 3 services (fetcher, orch, brainatlas)
- ✅ pgvector extension enabled
- ✅ Batch tracking tables
- ✅ Query storage tables
- ✅ Embeddings storage

### External Integrations (100%)
- ✅ AWS S3 client (self-hosted compatible)
- ✅ OpenRouter LLM client (embeddings + summarization + query generation)
- ✅ Lazy initialization with environment variable configuration

### Brainatlas Processing (100%)
- ✅ Full `/process` endpoint implemented
- ✅ S3 file download
- ✅ Text chunking (1000 chars, 200 overlap)
- ✅ Parallel embedding generation
- ✅ LLM summarization
- ✅ Vector DB storage
- ✅ Content deduplication (SHA-256 hash)
- ✅ Transaction-safe insertion

### Orch Completion Watcher (100%)
- ✅ Batch-based processing
- ✅ Poll loop (checks for ready batches)
- ✅ Process loop (calls brainatlas)
- ✅ Retry logic with exponential backoff
- ✅ Status tracking in database

### API Layer Structure (100%)
- ✅ All trait definitions
- ✅ API → App → Services delegation
- ✅ Error handling with proper types

---

## 🔶 What Needs Implementation

### 1. Orch App Business Logic (6 methods)

**File:** `orch/crates/app/src/app.rs`

All methods currently have `todo!()` placeholders:

#### **a. `search_region(region_id: Uuid)`** 
**Priority:** HIGH  
**Estimated:** 2-3 hours

**Logic:**
```rust
1. Query brainatlas for summaries by region_id
   - If summaries exist → return them with status=DONE
   
2. If no summaries:
   - Check if active batch exists (get_active_batch)
   - If batch exists → return current batch status
   
3. If no batch:
   - Check if queries exist for this region
   - If no queries → generate via LLM (call llm_service.generate_queries)
   - Store queries in region_queries table
   
   - Create batch (create_batch)
   - For each query:
     - Call fetcher POST /api/queue/enqueue
     - Store task_id
   - Add task IDs to batch (add_tasks_to_batch)
   
4. Return SearchRegionResult with status
```

**Services methods needed:**
- Get summaries from brainatlas (need to add this trait method)
- `get_active_batch(region_id)`
- `get_queries(region_id)`
- `insert_queries(region_id, queries)`
- `create_batch(region_id, expected_count)`
- `add_tasks_to_batch(batch_id, task_ids)`
- HTTP client to call fetcher `/api/queue/enqueue`

---

#### **b. `get_region_status(region_id: Uuid)`**
**Priority:** HIGH  
**Estimated:** 1-2 hours

**Logic:**
```rust
1. Get active batch for region
   - If no batch → status = NOT_STARTED
   
2. If batch exists:
   - Check batch.status:
     - "collecting" → FETCHING
     - "ready" → LLM_QUEUED
     - "processing" → PROCESSING
     - "completed" → DONE
     - "failed" → FAILED
     
3. Get summaries count
4. Get fetch task details (fetch_task_ids from batch)

5. Return RegionStatusResult with:
   - status
   - batch_id
   - fetch_task_count
   - summaries_count
   - created_at, ready_at, processing_started_at, completed_at timestamps
```

**Services methods needed:**
- `get_active_batch(region_id)`
- Get summaries count from brainatlas

---

#### **c. `invalidate_region(region_id: Uuid, priority: Option<Priority>)`**
**Priority:** MEDIUM  
**Estimated:** 1 hour

**Logic:**
```rust
1. Get active batch (if exists)
   - If batch status = 'completed':
     - Set status = 'collecting' (reset)
   - If batch status = 'collecting' or 'ready':
     - Just update priority on fetch tasks
     
2. If no batch exists:
   - Create new batch
   - Enqueue fetch tasks
   
3. Update priority on all fetch tasks in batch
   - Call fetcher API to update priority (need to add this endpoint to fetcher)
   
4. Return InvalidateResult with new status
```

**Services methods needed:**
- `update_batch_status(batch_id, status)`
- HTTP client to update fetch task priority (fetcher API)

---

#### **d. `get_pipeline_stats()`**
**Priority:** LOW  
**Estimated:** 1 hour

**Logic:**
```rust
1. Count regions by status:
   - Query all batches
   - Group by status
   - For regions with no batch → count as NOT_STARTED
   
2. Return PipelineStatsResult with counts:
   - not_started
   - fetch_queued
   - fetching
   - llm_queued
   - processing
   - done
   - failed
```

**Services methods needed:**
- `get_batches_by_status(status)` (already exists)
- Count total regions

---

#### **e. `get_config()`**
**Priority:** LOW  
**Estimated:** 30 minutes

**Logic:**
```rust
1. Call infra.get_all_config(database_url)
2. Convert to Vec<ConfigEntry>
3. Return
```

**Services methods needed:**
- Already exists in infra: `get_all_config()`

---

#### **f. `update_config(entries: Vec<ConfigEntryUpdate>)`**
**Priority:** LOW  
**Estimated:** 30 minutes

**Logic:**
```rust
1. For each entry:
   - Call infra.update_config(database_url, key, value)
   
2. Get all config again
3. Return updated Vec<ConfigEntry>
```

**Services methods needed:**
- Already exists in infra: `update_config()`

---

### 2. Additional Infra Methods Needed

#### **In orch/crates/services/src/infra.rs:**

Add to `Infra` trait:
```rust
// Query brainatlas for summaries
async fn get_summaries_for_region(&self, region_id: i32) -> Result<Vec<RegionSummary>, Self::Error>;

// Update fetch task priority (call fetcher API)
async fn update_fetch_task_priority(&self, task_id: i64, priority: i32) -> Result<(), Self::Error>;

// Enqueue fetch task (call fetcher API)
async fn enqueue_fetch_task(&self, query: String, region_id: i32, priority: i32) -> Result<i64, Self::Error>;
```

#### **Implementation:**
- Use `HttpClient` trait to call brainatlas and fetcher APIs
- Add retry logic with backon

---

### 3. Fetcher API Addition (Optional but Recommended)

**File:** `fetcher-be/crates/cortexmap-be/src/server.rs`

Add endpoint:
```rust
PATCH /api/queue/tasks/{task_id}/priority
Body: { priority: i32 }
```

**Purpose:** Allow orch to bump priority on invalidation

---

### 4. Orch HTTP Server

**Status:** Not started  
**Estimated:** 2-3 hours

**Structure:**
```
orch/crates/server/
├── Cargo.toml
├── src/
    ├── main.rs      - Server setup, route registration
    └── server.rs    - HTTP handlers
```

**Routes to implement:**
```rust
POST   /orch/api/regions/search            → search_region
GET    /orch/api/regions/{id}/status       → get_region_status
POST   /orch/api/regions/{id}/invalidate   → invalidate_region
GET    /orch/api/pipeline/stats            → get_pipeline_stats
GET    /orch/api/config                    → get_config
PATCH  /orch/api/config                    → update_config
```

Similar structure to `brainatlas-be/crates/server/`.

---

### 5. Testing

**Integration Tests:**
- End-to-end flow: search → enqueue → process → complete
- Invalidation flow
- Config updates

**Manual Testing:**
- Start all three services (fetcher, brainatlas, orch)
- Trigger search for a region
- Verify batch creation
- Verify fetch tasks enqueued
- Verify processing completes
- Verify summaries stored

---

## Implementation Priority Order

### **Phase 1: Core Search Flow (4-6 hours)**
1. Implement `search_region()` - triggers batch creation
2. Add missing infra methods (brainatlas summaries query, fetcher enqueue)
3. Test: search → batch → enqueue → poll → process → complete

### **Phase 2: Status & Observability (2-3 hours)**
1. Implement `get_region_status()` - status derivation
2. Implement `get_pipeline_stats()` - overall metrics

### **Phase 3: Configuration (1 hour)**
1. Implement `get_config()` and `update_config()`

### **Phase 4: Invalidation (2 hours)**
1. Implement `invalidate_region()`
2. Add fetcher priority update endpoint (if needed)

### **Phase 5: HTTP Server (3 hours)**
1. Create server crate
2. Add routes and handlers
3. Wire error handling

### **Phase 6: Testing (4-6 hours)**
1. Integration tests
2. Manual end-to-end testing
3. Bug fixes

---

## Total Remaining Effort

**Core functionality:** ~12-15 hours  
**HTTP server + testing:** ~7-9 hours  
**Total:** ~20-24 hours

---

## Next Immediate Step

**Implement `search_region()` first** - it's the core user-facing functionality and will validate the entire batch creation flow.

Once that works, everything else is just observability and management.
