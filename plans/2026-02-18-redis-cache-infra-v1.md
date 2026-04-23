# Redis Cache Infrastructure

## Objective

Add a `RedisInfra` trait with `get`, `set` (with optional expiry) operations across all three services (`brainatlas-be`, `orch`, `fetcher-be`). Each service uses a **unique key prefix** to guarantee namespace isolation -- cache set by one service cannot be accessed or modified by another.

## Architecture Design

### Key Prefix Isolation

| Service | Prefix | Example Key |
|---------|--------|-------------|
| `brainatlas-be` | `ba:` | `ba:regions:all`, `ba:chunk:{uuid}:source` |
| `orch` | `orch:` | `orch:config:all`, `orch:region:{uuid}:name` |
| `fetcher-be` | `fetcher:` | `fetcher:task_stats`, `fetcher:s3:{key}` |

The prefix is baked into each service's concrete `RedisInfra` implementation at construction time. The trait itself is prefix-unaware -- callers pass bare keys and the implementation prepends the prefix.

### Trait Design (per-service trait crate)

```rust
#[async_trait]
pub trait RedisInfra: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn cache_get(&self, key: &str) -> Result<Option<String>, Self::Error>;
    async fn cache_set(&self, key: &str, value: &str, ttl: Option<Duration>) -> Result<(), Self::Error>;
    async fn cache_del(&self, key: &str) -> Result<(), Self::Error>;
}
```

### Crate: `redis` (from crates.io)

Use the `redis` crate with `tokio-comp` feature for async support and `connection-manager` for automatic reconnection. Add `serde_json` for serializing complex types to cache values.

### Connection: `REDIS_URL` env var

Read from `EnvInfra` at startup, e.g. `redis://redis:6379`. In Docker, this points to the shared Redis on `infra-net`.

---

## Implementation Plan

### Phase 1: Infrastructure Setup

- [ ] **1.1** Add Redis service to `docker-compose.app.yml` on `infra-net` (image: `redis:7-alpine`, port 6379, healthcheck via `redis-cli ping`)
- [ ] **1.2** Add Redis service to `docker-compose.test.yml` for integration tests
- [ ] **1.3** Add `REDIS_URL` env var to all three service entries in `docker-compose.app.yml` (default: `redis://redis:6379`)

### Phase 2: brainatlas-be Redis Integration

- [ ] **2.1** Add `redis = { version = "0.29", features = ["tokio-comp", "connection-manager"] }` to `brainatlas-be/Cargo.toml` workspace deps and `brainatlas-be/crates/infra/Cargo.toml`
- [ ] **2.2** Add `RedisInfra` trait to `brainatlas-be/crates/services/src/infra.rs` with `cache_get`, `cache_set`, `cache_del` methods
- [ ] **2.3** Add `RedisInfra` to the blanket `Infra` trait bound in `brainatlas-be/crates/services/src/infra.rs`
- [ ] **2.4** Add `Redis` error variant to `brainatlas-be/crates/infra/src/error.rs`
- [ ] **2.5** Create `brainatlas-be/crates/infra/src/redis.rs` implementing concrete `BrainAtlasRedis` struct with:
  - Constructor taking `REDIS_URL` from env and prefix `"ba:"`
  - `ConnectionManager` for auto-reconnect
  - All three trait methods with automatic key prefixing
- [ ] **2.6** Wire `BrainAtlasRedis` into `BrainAtlasInfra` struct (`infra.rs`) -- read `REDIS_URL` from env at startup, construct `BrainAtlasRedis`, delegate `RedisInfra` impl
- [ ] **2.7** Register `redis` module in `brainatlas-be/crates/infra/src/lib.rs`

#### brainatlas-be Cache Points

- [ ] **2.8** `list_brain_regions()` -- cache in `BrainAtlasListBrainRegions::list()` (`services/src/list_brain_regions.rs`). Key: `regions:all`, TTL: 1 hour. The region mapping table (~1300 Allen Brain Atlas regions) is static reference data.
- [ ] **2.9** `search_brain_region(uuid)` -- cache in `BrainAtlasRegionInfo::search()` (`services/src/region_info.rs`). Key: `region:{uuid}:entries`, TTL: 15 min. Invalidate when `insert_summary_with_embeddings` or `update_summary_text` is called for the same region.
- [ ] **2.10** `get_chunk_source(chunk_id)` -- cache in `BrainAtlasServices::get_chunk_source()` (`services/src/services.rs:204-217`). Key: `chunk:{chunk_id}:source`, TTL: 24 hours. Immutable once created.
- [ ] **2.11** `check_content_hash(region_id, hash)` -- cache in `BrainAtlasServices::check_content_hash()` (`services/src/services.rs:131-140`). Key: `hash:{region_id}:{hash}`, TTL: 30 min. Deduplication check -- if hash is found, entire processing pipeline is skipped.
- [ ] **2.12** S3 downloads -- cache in `BrainAtlasServices::download()` (`services/src/services.rs:117-119`). Key: `s3:{key}`, TTL: 12 hours. S3 objects are immutable once uploaded.
- [ ] **2.13** Cache invalidation on writes: in `insert_summary_with_embeddings()` (`services.rs:142-169`), delete `region:{uuid}:entries` and `hash:{region_id}:*` pattern. In `update_summary_text()` (`services.rs:188-202`), delete `region:{uuid}:entries`.

### Phase 3: orch Redis Integration

- [ ] **3.1** Add `redis` dependency to `orch/Cargo.toml` workspace deps and `orch/crates/infra/Cargo.toml`
- [ ] **3.2** Add `RedisInfra` trait to `orch/crates/services/src/infra.rs` with same signature
- [ ] **3.3** Add `RedisInfra` to the blanket `Infra` trait bound in `orch/crates/services/src/infra.rs`
- [ ] **3.4** Add `Redis` error variant to `orch/crates/infra/src/error.rs`
- [ ] **3.5** Create `orch/crates/infra/src/redis.rs` implementing `OrchRedis` with prefix `"orch:"`
- [ ] **3.6** Wire into `OrchInfra` struct (`infra.rs`)
- [ ] **3.7** Register `redis` module in `orch/crates/infra/src/lib.rs`

#### orch Cache Points

- [ ] **3.8** `get_all_config()` -- cache in `OrchConfigManagement::get_all_config()` (`services/src/config_management.rs:25-50`). Key: `config:all`, TTL: 5 min. Invalidate on `update_config()`.
- [ ] **3.9** `get_config(key)` -- cache in `CompletionWatcher::get_config()` (`services/src/completion_watcher.rs:187-197`). Key: `config:{key}`, TTL: 60s. Called every poll cycle. Invalidate on `update_config()`.
- [ ] **3.10** `get_all_regions()` -- cache in `OrchRegionManagement::get_all_regions()` (`services/src/region_management.rs:279-308`). Key: `regions:all`, TTL: 1 hour. Static reference data.
- [ ] **3.11** `get_region_name(region_id)` -- cache in `OrchRegionManagement::get_region_name()` (`services/src/region_management.rs:224-238`). Key: `region:{uuid}:name`, TTL: 24 hours. Static reference data.
- [ ] **3.12** `get_total_regions()` -- cache in `OrchRegionManagement::get_total_regions()` (`services/src/region_management.rs:240-250`). Key: `regions:total`, TTL: 1 hour. Static.
- [ ] **3.13** `get_summaries(region_id)` -- cache in `OrchRegionManagement::get_summaries()` (`services/src/region_management.rs:26-76`). Key: `region:{uuid}:summaries`, TTL: 15 min. Invalidate when batch completes for that region.
- [ ] **3.14** `get_queries(region_id)` -- cache in `OrchRegionManagement::get_queries()` (`services/src/region_management.rs:108-118`). Key: `region:{uuid}:queries`, TTL: 10 min. Invalidate on `store_queries` or `delete_queries`.
- [ ] **3.15** `count_regions_without_batches()` -- cache in `OrchRegionManagement` (`services/src/region_management.rs:252-262`). Key: `regions:without_batches`, TTL: 60s. Invalidate on `create_batch`.
- [ ] **3.16** `get_query_generation_limit()` -- cache in `OrchRegionManagement` (`services/src/region_management.rs:264-277`). Key: `config:query_generation_limit`, TTL: 5 min. Invalidate with config.
- [ ] **3.17** `get_chunk_source(chunk_id)` -- cache in `OrchRegionManagement::get_chunk_source()` (`services/src/region_management.rs:322-363`). Key: `chunk:{chunk_id}:source`, TTL: 24 hours. Immutable.
- [ ] **3.18** Config invalidation: in `update_config()` (`config_management.rs:52-78`), delete `config:*` pattern to bust all config caches.
- [ ] **3.19** Query invalidation: in `store_queries()` and `delete_queries()`, delete `region:{uuid}:queries`.
- [ ] **3.20** Batch completion invalidation: when `complete_batch()` is called in `CompletionWatcher::process_batch()` (`completion_watcher.rs:422-427`), delete `region:{region_uuid}:summaries`.

### Phase 4: fetcher-be Redis Integration

- [ ] **4.1** Add `redis` dependency to `fetcher-be/Cargo.toml` workspace deps and `fetcher-be/crates/std-infra/Cargo.toml`
- [ ] **4.2** Add `RedisInfra` trait to `fetcher-be/crates/cortexmap-infra/src/infra.rs`
- [ ] **4.3** Add `Redis` error variant to `fetcher-be/crates/cortexmap-infra/src/error.rs`
- [ ] **4.4** Create `fetcher-be/crates/std-infra/src/redis.rs` implementing `FetcherRedis` with prefix `"fetcher:"`
- [ ] **4.5** Wire into `StdInfra` struct and implement `RedisInfra` delegation
- [ ] **4.6** Register `redis` module in `fetcher-be/crates/std-infra/src/lib.rs`

#### fetcher-be Cache Points

fetcher-be is **write-heavy** (task queue management, S3 uploads, paper insertions). Most operations are transactional state transitions that must not be cached. However:

- [ ] **4.7** `get_task_stats()` -- cache in `StdTaskQueue` or at the server handler level (`server.rs:181-312`). Key: `task_stats`, TTL: 10s. The status dashboard calls this frequently but exact real-time accuracy isn't critical.
- [ ] **4.8** `get_component_stats()` -- same pattern. Key: `component_stats`, TTL: 10s.
- [ ] **4.9** `get_task_by_pmc_id(pmc_id)` for **completed** tasks only -- Key: `task:{pmc_id}`, TTL: 5 min. Once a task is completed its state is final.
- [ ] **4.10** S3 content reads in `get_queue_status_handler` (`server.rs:207-247`) -- recent task summaries/abstracts are fetched from S3 on every status call. Key: `s3:{key}`, TTL: 1 hour. Content is immutable.

### Phase 5: Docker & Env

- [ ] **5.1** Add `REDIS_URL` to `.env.example` files for all services
- [ ] **5.2** Update all three Dockerfiles to add no new build args (Redis URL is runtime-only, consistent with our env infra pattern)
- [ ] **5.3** Verify `REDIS_URL` is read through `EnvInfra` (not raw `std::env::var`) in all services

### Phase 6: Graceful Degradation

- [ ] **6.1** All cache operations must be **non-fatal**. If Redis is down, log a warning and fall through to the database/S3. The `cache_get` failure path returns `Ok(None)`, and `cache_set`/`cache_del` failures are logged but do not propagate errors to callers.
- [ ] **6.2** Add `tracing::warn!` on Redis connection failures with a rate-limited approach (don't spam logs).

## Verification Criteria

- All three services compile with zero warnings
- `cargo test --lib` passes for all three workspaces
- Redis cache is transparent: if Redis is unavailable, services operate normally (just slower)
- brainatlas-be cache keys start with `ba:`, orch with `orch:`, fetcher with `fetcher:`
- `docker-compose.app.yml` includes Redis service and `REDIS_URL` env for all three services
- Write operations properly invalidate affected cache keys
- No service can read another service's cache keys (enforced by prefix at the implementation level)

## What NOT to Cache (Explicitly)

| Service | Operation | Reason |
|---------|-----------|--------|
| brainatlas-be | `summarize_with_tools` | Non-deterministic LLM output |
| brainatlas-be | `generate_queries` | Non-deterministic LLM output |
| brainatlas-be | `search_similar` | Unique embedding vectors per query; negligible hit rate |
| brainatlas-be | All write ops (`insert_*`, `update_*`) | Mutations |
| orch | `poll()` / `process()` | State-machine transitions; must see current DB state |
| orch | `get_active_batch` / `get_recent_batch` | Status changes constantly during pipeline |
| orch | `get_batches_by_status` | Volatile operational state |
| orch | `enqueue_fetch_task` / `ensure_workers_allocated` | Side-effecting external calls |
| fetcher-be | All task queue state transitions | Concurrent workers; must use DB-level locking |
| fetcher-be | `get_next_pending_task` | Uses `FOR UPDATE SKIP LOCKED`; stale data = lost tasks |

## Potential Risks and Mitigations

1. **Redis unavailability during startup**
   Mitigation: `ConnectionManager` auto-reconnects. Constructor should not fail if Redis is down -- use lazy connection. Log warning and proceed.

2. **Stale cache after writes**
   Mitigation: Every write path has explicit invalidation steps documented above. Use `cache_del` immediately after successful DB writes.

3. **Key collision between services**
   Mitigation: Prefix is hardcoded in each service's concrete Redis struct constructor. Not configurable -- eliminates misconfiguration risk.

4. **Memory pressure on Redis**
   Mitigation: All cached values have TTLs. Use `maxmemory-policy allkeys-lru` in Redis config. Most cached data is small (JSON strings, counts).

5. **Serialization overhead for complex types**
   Mitigation: Use `serde_json::to_string` / `from_str` for domain types. Only cache types that derive `Serialize`/`Deserialize` (most already do).

## Alternative Approaches

1. **In-process cache (DashMap/moka) instead of Redis**: Simpler, no network hop, but cache is per-process and lost on restart. Doesn't help with multi-container deployments. Good for static data (region mapping), bad for shared state.

2. **Shared cache crate across services**: A single `cache` crate in the repo root that all three services depend on. Reduces code duplication but creates coupling between otherwise-independent workspaces. Not recommended given the prefix isolation requirement.

3. **Redis Cluster/Sentinel**: Overkill for current scale. Single Redis instance is sufficient. Can migrate later if needed.
