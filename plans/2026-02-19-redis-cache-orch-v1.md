# Redis Cache Integration for Orch Service

## Objective

Introduce Redis as a caching layer in the `orch` service to reduce PostgreSQL load, improve response latency on read-heavy endpoints, and implement proper cache invalidation when data changes. The cache must integrate cleanly with the existing hexagonal architecture (infra -> services -> app -> api -> server) without disrupting the current trait-based design.

## Architecture Assessment

### Current State
- **Zero caching** exists anywhere in the project (`FIXME.md:19` lists `[ ] Redis cache` as a TODO)
- Every API call hits PostgreSQL directly, including the `get_all_regions` endpoint that queries 1000+ rows from `region_mapping` on every call
- `get_pipeline_stats` runs **5 separate** `get_batches_by_status` DB queries per request
- Domain types already implement `Serialize`/`Deserialize` (via serde), making Redis serialization straightforward
- The hexagonal architecture cleanly separates infra from business logic via traits defined in `orch/crates/services/src/infra.rs`

### Caching Candidates (Prioritized)

| Route | Handler | Cache Key Pattern | TTL | Invalidation Trigger | Priority |
|---|---|---|---|---|---|
| `GET /orch/api/regions` | `get_all_regions_handler` | `orch:regions:all` | Long (10m+) | Region mapping rarely changes | **HIGH** - Called on every frontend load, 1000+ rows |
| `GET /orch/api/pipeline/stats` | `get_pipeline_stats_handler` | `orch:pipeline:stats` | Short (15-30s) | Batch status changes | **HIGH** - 5 DB queries per call |
| `GET /orch/api/regions/{id}/summaries` | `list_summaries_handler` | `orch:region:{id}:summaries` | Medium (2-5m) | New summary generated (batch completion) | **HIGH** - Heavy query with JOINs |
| `GET /orch/api/regions/{id}/status` | `get_region_status_handler` | `orch:region:{id}:status` | Short (15-30s) | Batch status changes | **MEDIUM** - Polled by frontend |
| `GET /orch/api/batches/{id}/status` | `get_batch_status_handler` | `orch:batch:{id}:status` | Short (10-15s) | Batch status changes | **MEDIUM** - Polled during generation |
| `GET /orch/api/config` | `get_config_handler` | `orch:config:all` | Medium (2-5m) | Config update | **LOW** - Rarely called |
| `GET /orch/api/chunks/{id}/source` | `get_chunk_source_handler` | `orch:chunk:{id}:source` | Long (30m+) | Immutable once created | **LOW** - Proxied to brainatlas |
| `GET /orch/api/workers/status` | `get_worker_status_handler` | `orch:workers:status` | Very short (5s) | Real-time proxy, minimal benefit | **SKIP** |

### Routes NOT to Cache
- `POST /orch/api/regions/{id}/generate` — write operation (triggers batch creation)
- `PATCH /orch/api/config` — write operation (updates config)
- `POST /orch/api/workers/allocate` — write operation (allocates workers)
- `POST /orch/api/workers/stop` — write operation (stops workers)
- `GET /orch/health` — must always be live

### Invalidation Events

| Event | Keys to Invalidate |
|---|---|
| Batch status change (`update_batch_status`, `complete_batch`) | `orch:batch:{id}:status`, `orch:region:{region_id}:status`, `orch:pipeline:stats` |
| New summary generated (batch completed) | `orch:region:{region_id}:summaries`, `orch:region:{region_id}:status`, `orch:pipeline:stats` |
| Config updated (`update_config`) | `orch:config:all` |
| Region invalidated (`invalidate_region`) | `orch:region:{id}:summaries`, `orch:region:{id}:status`, `orch:pipeline:stats` |
| Generate summary triggered | `orch:region:{id}:status`, `orch:pipeline:stats` |

## Implementation Plan

### Phase 1: Infrastructure — Redis Client in `infra` Crate

- [x] **1.1 Add Redis dependencies to workspace `Cargo.toml`** (`orch/Cargo.toml`). Add `redis = { version = "0.27", features = ["tokio-comp", "connection-manager"] }` to `[workspace.dependencies]`. The `connection-manager` feature provides a reconnecting, clone-friendly async connection that works well with Axum's shared state.

- [x] **1.2 Add `redis` dependency to `infra` crate** (`orch/crates/infra/Cargo.toml`). Add `redis.workspace = true` to its `[dependencies]`.

- [x] **1.3 Add `redis` dependency to `services` crate** (`orch/crates/services/Cargo.toml`). Add `redis.workspace = true` since the cache trait will be defined in the services crate alongside the other infra traits.

- [x] **1.4 Define the `CacheClient` trait in `services/src/infra.rs`**. Add a new async trait alongside the existing `EnvInfra`, `HttpClient`, `OrchDatabase`, etc. The trait should define:
    - `async fn cache_get(&self, key: &str) -> Result<Option<String>, Self::Error>` — fetch a cached value
    - `async fn cache_set(&self, key: &str, value: &str, ttl_secs: u64) -> Result<(), Self::Error>` — set with TTL
    - `async fn cache_del(&self, key: &str) -> Result<(), Self::Error>` — delete a single key
    - `async fn cache_del_pattern(&self, pattern: &str) -> Result<u64, Self::Error>` — delete keys matching a glob pattern (for `orch:region:{id}:*` invalidation)
    
    This trait must use the same associated `Error` type pattern as the existing traits.

- [x] **1.5 Add `CacheClient` to the `Infra` supertrait bound** in `services/src/infra.rs:309-317`. Extend the `Infra` blanket trait to require `+ CacheClient<Error = <Self as Infra>::Error>`. Update the blanket `impl` accordingly.

- [x] **1.6 Create `orch/crates/infra/src/redis.rs`** — the concrete Redis implementation. Use `redis::aio::ConnectionManager` (auto-reconnecting, `Clone + Send + Sync`). Initialize lazily using `tokio::sync::OnceCell` (same pattern as `OrchPostgresql` at `pg.rs:18-37`). Read `REDIS_URL` env var (default `redis://127.0.0.1:6379`). Implement `CacheClient for OrchRedis`. For `cache_del_pattern`, use `SCAN` + `DEL` (not `KEYS` which blocks in production).

- [x] **1.7 Add `InfraError::Redis` variant** to `orch/crates/infra/src/error.rs`. Add `#[error("Redis error: {0}")] Redis(#[from] redis::RedisError)` to the `InfraError` enum.

- [x] **1.8 Integrate `OrchRedis` into `OrchInfra` struct** (`orch/crates/infra/src/infra.rs:14-34`). Add a `redis: OrchRedis` field to `OrchInfra`, construct it in `new()`, and implement the `CacheClient` trait delegation (same delegation pattern as `HttpClient`/`OrchDatabase`).

- [x] **1.9 Register the `redis` module** in `orch/crates/infra/src/lib.rs`. Add `mod redis;` alongside the existing modules.

### Phase 2: Cache Key Constants and Helpers

- [x] **2.1 Create cache key module in `services` crate** (`orch/crates/services/src/cache_keys.rs`). Define a module with functions that produce consistent cache keys:
    - `pub fn all_regions() -> String` → `"orch:regions:all"`
    - `pub fn pipeline_stats() -> String` → `"orch:pipeline:stats"`
    - `pub fn region_summaries(id: Uuid) -> String` → `format!("orch:region:{}:summaries", id)`
    - `pub fn region_status(id: Uuid) -> String` → `format!("orch:region:{}:status", id)`
    - `pub fn batch_status(id: Uuid) -> String` → `format!("orch:batch:{}:status", id)`
    - `pub fn config_all() -> String` → `"orch:config:all"`
    - `pub fn chunk_source(id: Uuid) -> String` → `format!("orch:chunk:{}:source", id)`
    - `pub fn region_pattern(id: Uuid) -> String` → `format!("orch:region:{}:*", id)` (for invalidation)
    
    Also define TTL constants:
    - `pub const TTL_LONG: u64 = 600;` (10 minutes — regions, chunk sources)
    - `pub const TTL_MEDIUM: u64 = 120;` (2 minutes — summaries, config)
    - `pub const TTL_SHORT: u64 = 15;` (15 seconds — stats, batch status, region status)

- [x] **2.2 Register the module** in `orch/crates/services/src/lib.rs`. Add `pub mod cache_keys;`.

### Phase 3: Cache-Through Reads in Service Layer

The caching logic belongs in the service implementations (not the app layer or infra layer). Each service method that reads cached data follows the pattern: check cache -> return if hit -> query DB -> serialize to cache -> return.

- [x] **3.1 Add caching to `OrchRegionManagement::get_all_regions`** (`orch/crates/services/src/region_management.rs:279-308`). Before the DB query, check `cache_get(cache_keys::all_regions())`. On hit, deserialize with `serde_json::from_str` and return. On miss, execute the existing DB query, then `cache_set` the serialized result with `TTL_LONG`. This is the highest-impact change since this returns 1000+ rows and is called on every frontend page load.

- [x] **3.2 Add caching to `OrchRegionManagement::get_summaries`** (`orch/crates/services/src/region_management.rs:26-76`). Cache key: `cache_keys::region_summaries(region_id)`. TTL: `TTL_MEDIUM`. This involves a region_mapping lookup + region_summary query + per-summary source chunk queries, making it expensive.

- [x] **3.3 Add caching to `OrchRegionManagement::get_batches_by_status`** (`orch/crates/services/src/region_management.rs:209-222`). Cache key: `format!("orch:batches:status:{}", status.as_str())`. TTL: `TTL_SHORT`. This is called 5 times by `get_pipeline_stats`, so caching it individually allows the 5 calls to share cached values within the TTL window.

- [x] **3.4 Add caching to `OrchBatchOrchestration::get_batch_by_id`** (`orch/crates/services/src/batch_orchestration.rs:191-201`). Cache key: `cache_keys::batch_status(batch_id)`. TTL: `TTL_SHORT`. Polled frequently by the frontend during active generation.

- [x] **3.5 Add caching to `OrchConfigManagement::get_all_config`** (`orch/crates/services/src/config_management.rs:25-49`). Cache key: `cache_keys::config_all()`. TTL: `TTL_MEDIUM`. Low-traffic but easy win.

- [x] **3.6 Add caching to `OrchRegionManagement::get_chunk_source`** (`orch/crates/services/src/region_management.rs:322-363`). Cache key: `cache_keys::chunk_source(chunk_id)`. TTL: `TTL_LONG`. Once a chunk source is resolved, it never changes. This also avoids the HTTP proxy call to brainatlas-be.

### Phase 4: Cache Invalidation on Writes

Each write operation must invalidate the relevant cached data. This is done by calling `cache_del` or `cache_del_pattern` after the write succeeds.

- [x] **4.1 Invalidate on batch status change** — In `OrchRegionManagement::update_batch_status` (`region_management.rs:120-135`), after the successful DB update, invalidate:
    - `cache_keys::batch_status(batch_id)` 
    - `cache_keys::pipeline_stats()`
    - All per-status batch caches: `cache_del_pattern("orch:batches:status:*")`
    
    Note: `region_id` is not available in the current method signature. Either look up the batch to get its `region_id` for region-level invalidation, or accept that `TTL_SHORT` on region status will naturally expire. The pragmatic approach is to also invalidate `orch:region:*:status` via pattern.

- [x] **4.2 Invalidate on batch completion** — The `CompletionWatcher::process_batch` method (`completion_watcher.rs:236-457`) calls `complete_batch` on success. After the `complete_batch` call succeeds (`completion_watcher.rs:422-427`), invalidate:
    - `cache_keys::region_summaries(batch.region_id)` — new summary was generated
    - `cache_keys::region_status(batch.region_id)`
    - `cache_keys::batch_status(batch.id)`
    - `cache_keys::pipeline_stats()`
    - `"orch:batches:status:*"` pattern

- [x] **4.3 Invalidate on config update** — In `OrchConfigManagement::update_config` (`config_management.rs:52-78`), after the loop of `update_config` calls, invalidate `cache_keys::config_all()`.

- [x] **4.4 Invalidate on generate summary** — In `OrchApp::generate_summary` (`app.rs:209-284`), the batch creation and task enqueue should invalidate:
    - `cache_keys::region_status(region_id)`
    - `cache_keys::pipeline_stats()`
    - `"orch:batches:status:*"` pattern
    
    This requires the app layer to have access to cache invalidation. Two approaches:
    - **Option A (Recommended)**: Add an `invalidate_cache` method to the `Services` trait so the app calls `self.services.invalidate_region_cache(region_id).await`
    - **Option B**: Perform invalidation in the service layer methods that are called by `generate_summary` (e.g., `create_batch`, `add_tasks_to_batch`)

- [x] **4.5 Invalidate on region invalidation** — In `OrchApp::invalidate_region` (`app.rs:121-152`), after marking the batch as invalidated, invalidate:
    - `cache_keys::region_summaries(region_id)`
    - `cache_keys::region_status(region_id)`
    - `cache_keys::pipeline_stats()`
    - `"orch:batches:status:*"` pattern

### Phase 5: Infrastructure & Docker

- [x] **5.1 Add Redis service to `docker-compose.app.yml`**. Add a `redis` service using `redis:7-alpine` image with a healthcheck (`redis-cli ping`), persistent volume mount, and connection to `infra-net`. Add `REDIS_URL: redis://redis:6379` to the orch service's environment block.

- [x] **5.2 Add Redis service to `docker-compose.test.yml`**. Add a `redis-test` service on port 6380 (to avoid collision with any local Redis) for integration tests.

- [x] **5.3 Add `REDIS_URL` env var support** in `OrchRedis` (created in 1.6). Read from `REDIS_URL` env var with fallback to `redis://127.0.0.1:6379`. This should use the same `EnvInfra` trait pattern but can also direct-read since the redis module is in infra.

### Phase 6: Graceful Degradation & Error Handling

- [x] **6.1 Cache operations must never fail the request**. All `cache_get` misses (including Redis connection failures) should fall through to the database query. All `cache_set` failures should be logged at `warn` level but not propagate errors to the caller. The service methods should wrap cache operations in a helper or use `.unwrap_or(None)` / `.ok()` patterns.

- [x] **6.2 Add a generic `cached_or_fetch` helper** in the services crate that encapsulates the read-through pattern:
    ```
    async fn cached_or_fetch<T>(cache, key, ttl, fetch_fn) -> Result<T, E>
    ```
    This helper: tries `cache_get` (swallowing errors) -> deserialize -> on miss, calls `fetch_fn` -> serialize -> `cache_set` (swallowing errors) -> return. This avoids duplicating the cache-through logic in every service method.

- [x] **6.3 Add cache-miss metrics logging**. In the `cached_or_fetch` helper, log at `debug` level whether each call was a cache hit or miss. This enables monitoring cache effectiveness via structured logging.

### Phase 7: Update FIXME.md

- [x] **7.1 Mark the Redis cache TODO as complete** in `FIXME.md:19`. Change `- [ ] Redis cache` to `- [x] Redis cache`.

## Verification Criteria

- All existing integration tests pass without Redis running (graceful degradation)
- `GET /orch/api/regions` returns cached response on second call within TTL (verify via Redis `EXISTS` or response time)
- `GET /orch/api/pipeline/stats` hits cache after first call (5 DB queries reduced to 0-1)
- `POST /orch/api/regions/{id}/generate` invalidates the region's status and pipeline stats caches
- `PATCH /orch/api/config` invalidates the config cache
- Batch completion (via completion watcher) invalidates region summaries and stats caches
- Redis connection failure does not cause 500 errors — requests fall through to PostgreSQL
- Docker Compose brings up Redis alongside other services and orch connects successfully
- No changes to the `domain` crate (pure types remain pure)
- No changes to the `api` or `server` crate signatures (caching is transparent to the HTTP layer)

## Potential Risks and Mitigations

1. **Stale cache serving incorrect data during rapid state transitions (e.g., batch going from `collecting` -> `ready` -> `processing` in quick succession)**
   Mitigation: Use short TTLs (15s) for status endpoints and always invalidate on write. The completion watcher runs every 30s by default, so a 15s TTL ensures at most one stale response between polls.

2. **Redis connection failure at startup preventing orch from starting**
   Mitigation: Initialize Redis lazily (OnceCell pattern matching PostgreSQL) and make all cache operations non-fatal. If Redis is unavailable, the service operates identically to today (no cache).

3. **Memory pressure on Redis from large payloads (e.g., `get_all_regions` with 1000+ entries)**
   Mitigation: The `region_mapping` table has ~1300 rows. Serialized as JSON, this is ~200-400KB — well within Redis defaults. Set `maxmemory-policy allkeys-lru` in Redis config for safety.

4. **Cache key collision or inconsistency across deployments**
   Mitigation: Prefix all keys with `orch:` namespace. The key module centralizes all key generation, preventing ad-hoc key construction.

5. **`SCAN`-based pattern deletion being slow with many keys**
   Mitigation: The total number of cached keys is small (< 2000 even with per-region caches for all 1300 regions). SCAN with a reasonable COUNT hint (100) will complete in 1-2 iterations.

6. **Trait explosion — adding `CacheClient` to `Infra` requires updating all test mocks**
   Mitigation: The `CacheClient` trait has a simple 4-method surface. For tests, provide a no-op implementation that always returns `Ok(None)` / `Ok(())`. This can be a `NoopCache` struct in the test utilities.

## Alternative Approaches

1. **In-process cache (e.g., `moka` crate) instead of Redis**: Simpler to deploy (no external service), but doesn't share cache across multiple orch instances if horizontally scaled. Currently there's a single orch instance, so this would work today. However, Redis is already on the FIXME list explicitly, provides persistence across restarts, and positions the system for multi-instance deployment. **Recommendation: Use Redis as planned, but the `CacheClient` trait abstraction would allow swapping to moka later.**

2. **Axum middleware-level HTTP caching (ETag/Cache-Control headers)**: Would cache at the HTTP transport layer. Simpler but less granular — can't invalidate individual regions when a batch completes. Also doesn't help the completion watcher's internal DB queries. **Recommendation: Not sufficient alone, but could be added later as a complementary layer.**

3. **Cache in the `app` layer instead of the `services` layer**: The `OrchApp` struct has access to `Services` trait. Adding cache there would centralize logic but break the hexagonal architecture principle (app should contain business logic, not infrastructure concerns). **Recommendation: Cache in services layer, expose invalidation helpers via the `Services` trait for the app layer to call on write paths.**

## Files Modified (Summary)

| File | Change |
|---|---|
| `orch/Cargo.toml` | Add `redis` workspace dependency |
| `orch/crates/infra/Cargo.toml` | Add `redis.workspace = true` |
| `orch/crates/services/Cargo.toml` | Add `redis.workspace = true` (for trait) |
| `orch/crates/infra/src/lib.rs` | Add `mod redis;` |
| `orch/crates/infra/src/error.rs` | Add `Redis` variant |
| `orch/crates/infra/src/redis.rs` | **NEW** — `OrchRedis` struct + `CacheClient` impl |
| `orch/crates/infra/src/infra.rs` | Add `redis` field to `OrchInfra`, delegate `CacheClient` |
| `orch/crates/services/src/lib.rs` | Add `pub mod cache_keys;` |
| `orch/crates/services/src/cache_keys.rs` | **NEW** — Key generation functions + TTL constants |
| `orch/crates/services/src/infra.rs` | Add `CacheClient` trait, update `Infra` supertrait |
| `orch/crates/services/src/region_management.rs` | Add cache reads + invalidation |
| `orch/crates/services/src/batch_orchestration.rs` | Add cache reads + invalidation |
| `orch/crates/services/src/config_management.rs` | Add cache reads + invalidation |
| `orch/crates/services/src/completion_watcher.rs` | Add invalidation after batch completion |
| `docker-compose.app.yml` | Add Redis service + env var |
| `docker-compose.test.yml` | Add Redis test service |
| `FIXME.md` | Mark Redis cache as complete |
