# Redis Queue Migration — fetcher-be

## Objective

Replace the PostgreSQL-backed `StdTaskQueue` with a Redis Streams + Consumer Groups queue. Redis `XREADGROUP` / `XACK` / `XAUTOCLAIM` provide atomic claim, explicit acknowledgement, and automatic stale-task recovery. PostgreSQL retains its role for component tracking, logs, and the `fetch_tasks` state-mirror table (used for stats and the HTTP API). The `TaskQueueInfra` trait interface is preserved so callers (`worker.rs`, `server.rs`, `worker_manager.rs`) require only targeted updates.

---

## Background: Current vs Target Architecture

| Concern | Current | Target |
|---|---|---|
| Queue storage | `fetch_tasks` rows (`status='pending'`) | Redis Stream `fetcher:tasks` |
| Atomic claim | `SELECT … FOR UPDATE SKIP LOCKED` | `XREADGROUP GROUP fetcher:workers <worker_id>` |
| Acknowledgement | `mark_task_completed` / `mark_task_failed` updates `status` in PG | `XACK fetcher:tasks fetcher:workers <stream_id>` |
| Stale recovery | `reset_stale_tasks` / `release_stale_tasks_by_heartbeat` PG functions | `XAUTOCLAIM fetcher:tasks fetcher:workers <worker_id> <min-idle-ms> 0-0` |
| Heartbeat | `UPDATE fetch_tasks SET heartbeat_at = NOW()` | `SET fetcher:heartbeat:<task_stream_id> 1 EX <ttl>` — refreshed inside `worker_loop` |
| State mirror | `fetch_tasks.status` column | `HSET fetcher:task:<task_id> status … worker_id … heartbeat_at …` (Redis hash) + `fetch_tasks` row kept in sync for stats/API |
| Deduplication | `UNIQUE(pmc_id, query)` + `ON CONFLICT DO UPDATE` | Same PG constraint retained as authoritative; Redis entry added only after successful PG insert |
| Component tracking | `fetch_task_components` (PG) | Unchanged — stays in PostgreSQL |
| Logs | `fetch_task_logs` (PG) | Unchanged — stays in PostgreSQL |

---

## Redis Data Structures

### 1. Stream — `fetcher:tasks`
Primary queue. One message per enqueued task.

Fields per message:
```
task_id    <i64>
pmc_id     <String>
query      <String>
priority   <i32>
region_id  <i32 | "">
max_attempts <u32>
created_at <unix_timestamp_secs>
```

Consumer group: `fetcher:workers` (created once with `XGROUP CREATE fetcher:tasks fetcher:workers $ MKSTREAM`).

### 2. Task State Hash — `fetcher:task:{task_id}`
Mirrors live task state for low-latency lookups without hitting PostgreSQL on every heartbeat.

Fields: `status`, `stream_id` (the Redis message ID, needed for `XACK`), `worker_id`, `worker_version`, `started_at`, `completed_at`, `heartbeat_at`, `error`.

TTL: set to 7 days on completion/failure via `EXPIRE`.

### 3. Heartbeat Key — `fetcher:heartbeat:{stream_id}`
A simple `SET … EX <heartbeat_ttl_secs>` key refreshed by the worker every `heartbeat_interval`. If the key expires before `XAUTOCLAIM` runs, the task is eligible for re-delivery. The heartbeat TTL should be `2 × heartbeat_interval`.

### 4. Deduplication Key — `fetcher:dedup:{pmc_id}:{query_hash}`
Set with `SET NX EX 86400` (24h) at enqueue time. Prevents re-adding a pmc_id+query pair already in the stream.

---

## Implementation Plan

### Phase 1 — New Redis Infrastructure Layer

- [ ] Task 1.1. **Add `deadpool-redis` and `redis` crate dependencies** to `fetcher-be/Cargo.toml` workspace dependencies section. Use `redis` with features `tokio-comp, streams` and `deadpool-redis` with feature `rt_tokio_1`. Pin compatible versions (redis `0.27`, deadpool-redis `0.18` or latest compatible). Add both as workspace deps so all crates can inherit them.

- [ ] Task 1.2. **Add `REDIS_URL` to `FetcherEnvInfra` and `StdInfraContext`** in `crates/std-infra/src/lib.rs` and `crates/std-infra/src/env.rs`. `StdInfraContext::from_env()` must read the `REDIS_URL` environment variable (alongside the existing `DATABASE_URL`, S3 vars). No fallback default — fail fast with `InfraError::EnvVarNotFound` if absent.

- [ ] Task 1.3. **Create `crates/std-infra/src/redis_infra.rs`** — a new module housing the `RedisPool` type alias (`deadpool_redis::Pool`) and a `StdRedisInfra` struct with a single `pool: RedisPool` field. Expose a `pub fn new(redis_url: &str) -> Result<Self, InfraError>` constructor that builds a `deadpool_redis::Config` from the URL and creates the pool with a max size of 20 connections. Implement an `async fn get_conn(&self) -> Result<deadpool_redis::Connection, InfraError>` helper. Map `deadpool_redis::PoolError` to `InfraError::DatabaseError` (reuse existing variant) or introduce a new `InfraError::RedisError(String)` variant in `crates/cortexmap-infra/src/error.rs`.

- [ ] Task 1.4. **Add `StdRedisInfra` field to `StdInfra`** in `crates/std-infra/src/infra.rs`. Construct it in `StdInfra::new(…, redis_url: &str)`. Update `StdInfraContext::get()` to pass `redis_url` through. The existing `db_infra` and pool-sharing arrangement is untouched.

- [ ] Task 1.5. **Create `crates/std-infra/src/task_queue.rs` — `RedisTaskQueue` struct** replacing the existing `StdTaskQueue`. Fields: `redis: StdRedisInfra`, `pg_pool: DbPool` (kept for component/log operations). The struct implements the existing `TaskQueueInfra` trait from `crates/cortexmap-infra/src/infra.rs`.

---

### Phase 2 — `TaskQueueInfra` Trait Updates

- [ ] Task 2.1. **Add `stream_message_id: Option<String>` to `FetchTask` model** in `crates/cortexmap-infra/src/database/models.rs`. This field records the Redis stream message ID so any code holding a `FetchTask` can call `XACK` without a separate Redis lookup.

- [ ] Task 2.2. **Add new migration** `migrations/<timestamp>_add_stream_message_id/up.sql`:
  ```sql
  ALTER TABLE fetch_tasks ADD COLUMN stream_message_id TEXT;
  CREATE INDEX idx_fetch_tasks_stream_id ON fetch_tasks(stream_message_id)
    WHERE stream_message_id IS NOT NULL;
  ```
  Add corresponding `down.sql` dropping the column. Update `crates/cortexmap-infra/src/database/schema.rs` to include `stream_message_id -> Nullable<Text>`.

- [ ] Task 2.3. **Revise `TaskQueueInfra` trait in `crates/cortexmap-infra/src/infra.rs`** — retire three PostgreSQL-specific methods that are wholly replaced by Redis primitives:
  - Remove `reset_stale_tasks` (replaced by `reclaim_stale_tasks`).
  - Remove `release_stale_tasks_by_heartbeat` (subsumed by `reclaim_stale_tasks`).
  - Remove `release_worker_tasks` (subsumed by `reclaim_stale_tasks` targeting a specific consumer).
  - Add `async fn reclaim_stale_tasks(&self, min_idle_ms: u64) -> Result<Vec<FetchTask>, InfraError>` — wraps `XAUTOCLAIM`.
  - Add `async fn update_task_heartbeat_redis(&self, stream_id: &str, ttl_secs: u64) -> Result<(), InfraError>` — wraps `SET fetcher:heartbeat:{stream_id} 1 EX ttl`.
  - Retain all other methods unchanged so downstream callers compile without modification.

---

### Phase 3 — `RedisTaskQueue` Method Implementations

- [ ] Task 3.1. **`enqueue_task(pmc_id, query, max_attempts)`** — Two-step atomic enqueue:
  1. Check deduplication: `SET fetcher:dedup:{pmc_id}:{sha256(query)[..8]} {placeholder} NX EX 86400`. If `NX` fails, read existing `task_id` from `HGET fetcher:dedup-meta:{pmc_id}:{hash} task_id` and return the existing `FetchTask` from PostgreSQL.
  2. Insert into `fetch_tasks` via Diesel (`ON CONFLICT DO UPDATE SET updated_at = NOW()` — unchanged). Insert `fetch_task_components` rows — unchanged.
  3. On successful PG insert: `XADD fetcher:tasks * task_id <id> pmc_id <pmc_id> query <query> priority <priority> region_id <region_id|""> max_attempts <max_attempts> created_at <now_unix>`. Store the returned stream message ID into `HSET fetcher:task:<task_id> stream_id <msg_id> status pending`.
  4. Return the `FetchTask` with `stream_message_id` populated.
  - *Rationale:* PostgreSQL remains the authoritative deduplication and component-creation layer. Redis receives only successfully inserted tasks.

- [ ] Task 3.2. **`get_next_pending_task(timeout_secs)`** — Replace `FOR UPDATE SKIP LOCKED` with:
  `XREADGROUP GROUP fetcher:workers <worker_id> COUNT 1 BLOCK <timeout_secs * 1000> STREAMS fetcher:tasks >`
  On a message: deserialize fields from the stream entry, load the corresponding `FetchTask` from PostgreSQL by `task_id` (to get all model fields), set `stream_message_id` on the returned struct.
  On timeout (`nil`): return `Ok(None)`.
  - *Rationale:* `>` delivers only new, undelivered messages. Stale re-deliveries come through `reclaim_stale_tasks` separately.

- [ ] Task 3.3. **`claim_task_for_worker(task_id, worker_id, worker_version)`** — After `XREADGROUP` delivers a message, record ownership:
  `HSET fetcher:task:{task_id} status in_progress worker_id {worker_id} worker_version {worker_version} started_at {now}`
  Mirror to PostgreSQL: `UPDATE fetch_tasks SET status='in_progress', worker_id=?, worker_version=?, started_at=NOW(), heartbeat_at=NOW() WHERE id=?` (existing Diesel call, unchanged).

- [ ] Task 3.4. **`update_task_heartbeat` (both variants)** — The existing `update_task_heartbeat(task_id)` method updates the PostgreSQL row (keep for stats API compatibility). Add the new `update_task_heartbeat_redis(stream_id, ttl_secs)` that issues `SET fetcher:heartbeat:{stream_id} 1 EX {ttl_secs}`. Both are called from `worker_loop` on each heartbeat tick.

- [ ] Task 3.5. **`mark_task_completed(task_id)`** — Three operations:
  1. `XACK fetcher:tasks fetcher:workers {stream_id}` — removes from PEL, message stays in stream log.
  2. `HSET fetcher:task:{task_id} status completed completed_at {now}` then `EXPIRE fetcher:task:{task_id} 604800` (7 days).
  3. `DEL fetcher:heartbeat:{stream_id}` — clean up heartbeat key.
  4. Mirror to PostgreSQL: `UPDATE fetch_tasks SET status='completed', completed_at=NOW() WHERE id=?` (existing call, unchanged).
  The `stream_id` is taken from `task.stream_message_id` — now always populated.

- [ ] Task 3.6. **`mark_task_failed(task_id, error)`** — Same pattern as completion:
  1. `XACK` the stream message (it failed permanently; we do not want re-delivery).
  2. `HSET fetcher:task:{task_id} status failed error {error_msg}` + `EXPIRE`.
  3. `DEL fetcher:heartbeat:{stream_id}`.
  4. Mirror to PostgreSQL via the existing `log_task_event` + `UPDATE fetch_tasks SET status='failed'` calls.

- [ ] Task 3.7. **`release_task(task_id)`** — When a task finishes with only partial component completion (the `release_task` path in `worker.rs:248-257`):
  Do **not** `XACK` — leave the message in the PEL so it becomes eligible for `XAUTOCLAIM` after the heartbeat TTL expires, OR immediately re-inject via `XADD` with the same fields and reset `HSET fetcher:task:{task_id} status pending worker_id "" heartbeat_at ""`. Update PostgreSQL `fetch_tasks` accordingly.
  - *Design note:* Re-injecting via `XADD` (rather than relying on PEL re-delivery) gives immediate re-queueing without waiting for the idle timeout. The old PEL entry is then `XACK`-ed to clean up.

- [ ] Task 3.8. **`reclaim_stale_tasks(min_idle_ms)`** — Replaces `reset_stale_tasks` and `release_stale_tasks_by_heartbeat`:
  `XAUTOCLAIM fetcher:tasks fetcher:workers <calling_worker_id> <min_idle_ms> 0-0 COUNT 50`
  For each reclaimed entry: update `HSET fetcher:task:{task_id} worker_id {new_worker_id} heartbeat_at {now}` and mirror to PostgreSQL. Return the reclaimed `FetchTask` list. The caller (`worker_loop`) can immediately process them.

- [ ] Task 3.9. **`get_task_stats()` and `get_detailed_task_stats()`** — Augment with Redis data:
  - Pending count: `XLEN fetcher:tasks` minus the PEL size from `XINFO GROUPS fetcher:tasks` → `pending` field.
  - In-progress count: PEL size from `XINFO GROUPS`.
  - Completed/failed counts: still queried from PostgreSQL `fetch_tasks` (unchanged, as these are archived rows).
  This fixes the FIXME noted in `FIXME.md:8` — "In progress count is incorrect."

- [ ] Task 3.10. **All remaining `TaskQueueInfra` methods** (`get_pending_components`, `update_component_status`, `increment_component_attempt`, `all_components_completed`, `log_task_event`, `get_recent_tasks`, `get_task_by_pmc_id`, `get_task_by_id`, `get_tasks_by_status`, `get_task_components`) — **no functional change**. These all operate exclusively on PostgreSQL tables (`fetch_task_components`, `fetch_task_logs`, `fetch_tasks`). Copy implementations verbatim from existing `StdTaskQueue`; only the struct name changes.

---

### Phase 4 — Worker Loop Updates

- [ ] Task 4.1. **Add periodic heartbeat pulsing to `worker_loop`** in `crates/cortexmap-fetcher/src/worker.rs`. Wrap the `process_task` call in a `tokio::select!` that races between the task future and a `tokio::time::interval` ticker. On each tick, call `ctx.infra.update_task_heartbeat(task_id)` (PostgreSQL) and `ctx.infra.update_task_heartbeat_redis(stream_id, heartbeat_ttl_secs)`. The heartbeat interval should be sourced from `blueprint.fetcher.retry_config` (add `heartbeat_interval_secs: u64` field with default `15`). This fills the current gap noted in the research — `update_task_heartbeat` exists but is never called.

- [ ] Task 4.2. **Add a stale-task reclaim pass to `worker_loop`** — at the start of each polling iteration (before `get_next_pending_task`), call `reclaim_stale_tasks(min_idle_ms)` where `min_idle_ms = heartbeat_ttl_secs * 1000 * 2`. Reclaimed tasks are processed immediately in-loop before polling for new ones. This replaces the separate `reset_stale_tasks` utility function and the two PG stored-function calls.

- [ ] Task 4.3. **Propagate `stream_message_id` through `process_task`** — `process_task` currently receives a `FetchTask`. Ensure the `stream_message_id` field on that struct is forwarded into the `mark_task_completed`, `mark_task_failed`, and `release_task` calls so those implementations have the stream ID available for `XACK`.

- [ ] Task 4.4. **Update `worker_loop` exit/cancellation path** — on `tokio::select!` cancel signal, call `release_task(task.id)` for any in-progress task (not `mark_task_failed`) so it re-enters the queue rather than staying stuck in PEL. Then call `XACK` of the PEL entry only after the re-inject `XADD` completes, ensuring atomicity of the "give back + remove stale claim" operation.

---

### Phase 5 — `WorkerManager` and Server Updates

- [ ] Task 5.1. **Update `query_worker_stats` in `crates/cortexmap-be/src/worker_manager.rs`** — the current raw SQL queries `fetch_tasks WHERE worker_id=$1` for completed/failed counts and live heartbeat. Replace with:
  - Completed/failed counts: same PostgreSQL query (these rows stay in PG with `worker_id` intact — no change).
  - Live heartbeat: `HGET fetcher:task:{task_id} heartbeat_at` on the active task (from Redis hash), falling back to the PostgreSQL `heartbeat_at` column.
  - Current PMC ID: `HGET fetcher:task:{active_task_id} pmc_id` from Redis.

- [ ] Task 5.2. **Add Redis health check to `health_handler`** in `crates/cortexmap-be/src/server.rs`. Issue a `PING` to Redis. Return `503` if Redis is unreachable, matching the existing pattern for PostgreSQL pool health.

- [ ] Task 5.3. **Update `get_queue_status_handler`** — pending/in-progress counts should now read from Redis (`XLEN` + `XINFO GROUPS` PEL count) instead of PostgreSQL `COUNT WHERE status=…`. Completed/failed counts continue reading from PostgreSQL. This aligns with Task 3.9.

---

### Phase 6 — Consumer Group Bootstrap

- [ ] Task 6.1. **Create a `bootstrap_redis_queue` function** in `crates/std-infra/src/redis_infra.rs`. Called once at application startup (in `QueueServer::from_env()` or `StdInfra::new()`). Issues:
  `XGROUP CREATE fetcher:tasks fetcher:workers $ MKSTREAM`
  wrapped in an error handler that ignores `BUSYGROUP` error (group already exists). This is idempotent across restarts.

- [ ] Task 6.2. **Add `REDIS_URL` to `docker-compose.app.yml`** environment section for the `fetcher-be` service, pointing to the Redis container. Add a Redis service entry (`image: redis:7-alpine`, port `6379`) if not already present in the compose file. Add `redis` service dependency to `fetcher-be`'s `depends_on` list.

- [ ] Task 6.3. **Add `REDIS_URL` to `docker-compose.test.yml`** and `test.sh` for the integration test environment. Ensure the Redis instance is flushed between test runs (add `redis-cli FLUSHDB` to the test setup/teardown hooks in `setup-test-data.sh` or a new Redis-specific setup script).

---

### Phase 7 — Testing

- [ ] Task 7.1. **Rewrite `crates/std-infra/tests/task_queue_tests.rs`** for the Redis-backed implementation. Each test must:
  - Flush the Redis test DB (`FLUSHDB`) and delete test rows from `fetch_tasks` in setup/teardown.
  - Replace PostgreSQL-specific assertions (e.g., `stats.pending > 0`) with Redis-aware equivalents (e.g., `XLEN fetcher:tasks > 0`, `XINFO GROUPS` PEL count).
  - Port all 8 existing test scenarios (enqueue, deduplication, claim timeout, component updates, retry increment, stale reset, task completion, stats invariant) to the new primitives.

- [ ] Task 7.2. **Add new tests for Redis-specific behavior** in `task_queue_tests.rs`:
  - `test_xack_removes_from_pel` — verify that `mark_task_completed` causes PEL size to decrease by 1.
  - `test_xautoclaim_recovers_stale` — simulate a stale PEL entry (sleep past `min_idle_ms`) then call `reclaim_stale_tasks`; assert it returns the task.
  - `test_heartbeat_ttl_prevents_premature_reclaim` — verify that a task with a fresh heartbeat key is not returned by `reclaim_stale_tasks`.
  - `test_deduplication_via_nx` — verify that enqueuing the same `pmc_id`+`query` twice does not produce two stream entries.

- [ ] Task 7.3. **Update `crates/cortexmap-fetcher/tests/worker_integration_tests.rs`** — `test_concurrent_workers_no_duplicate` currently tests `FOR UPDATE SKIP LOCKED`. Replace with the equivalent Redis consumer group test: three concurrent `XREADGROUP` calls must each claim a distinct stream entry (no duplicate message IDs across consumers).

- [ ] Task 7.4. **Update `perf_tests.rs`** — replace the r2d2/Diesel pool latency benchmarks with equivalent Redis stream benchmarks. The `test_task_claiming_latency` benchmark should now measure `XREADGROUP` round-trip latency (expected to be significantly lower than the PostgreSQL `FOR UPDATE SKIP LOCKED` baseline).

---

### Phase 8 — Configuration and Cleanup

- [ ] Task 8.1. **Add `heartbeat_interval_secs: u64` (default `15`) and `heartbeat_ttl_secs: u64` (default `45`) fields to `RetryConfig`** in `crates/cortexmap-core/src/blueprint/connections/fetcher.rs`. Add `stale_reclaim_min_idle_ms: u64` (default `60000`, i.e. 60s) to replace the `stale_task_multiplier` heuristic.

- [ ] Task 8.2. **Remove the two PostgreSQL stored functions** `release_worker_tasks` and `release_stale_tasks` — add a migration `down.sql` step that drops them, and a new `up.sql` that drops them if they exist. These are no longer called by any Rust code.

- [ ] Task 8.3. **Remove `reset_stale_tasks` public function** from `crates/cortexmap-fetcher/src/worker.rs` (`worker.rs:470-482`) — it is now subsumed by the in-loop `reclaim_stale_tasks` call added in Task 4.2. Remove its re-export from `crates/cortexmap-fetcher/src/lib.rs`.

- [ ] Task 8.4. **Remove `release_worker_tasks`, `release_stale_tasks_by_heartbeat`, `reset_stale_tasks` methods** from the `TaskQueueInfra` trait and all `impl` blocks. These are replaced by `reclaim_stale_tasks` from Task 2.3.

- [ ] Task 8.5. **Update `crates/std-infra/src/lib.rs` `pub mod` list** to include `redis_infra` and remove any residual references to the old `StdTaskQueue` type.

---

## Verification Criteria

- All 8 original `task_queue_tests` pass against the Redis-backed `RedisTaskQueue` implementation.
- `test_concurrent_workers_no_duplicate` passes — three concurrent `XREADGROUP` claims produce three distinct, non-overlapping message IDs.
- `test_xack_removes_from_pel` passes — PEL size after `mark_task_completed` is 0.
- `test_xautoclaim_recovers_stale` passes — tasks with expired heartbeat keys are returned by `reclaim_stale_tasks`.
- `test_heartbeat_ttl_prevents_premature_reclaim` passes — a fresh heartbeat key blocks reclaim.
- `docker-compose up` starts successfully with the Redis service present; `GET /fetcher-be/health` returns `200`.
- The FIXME `"In progress count is incorrect"` (`FIXME.md:8`) is resolved: `get_queue_status_handler` returns the PEL count for `in_progress`, which is exact.
- No PG stored functions (`release_worker_tasks`, `release_stale_tasks`) are called anywhere in Rust code.
- `get_next_pending_task` no longer issues any `SELECT … FOR UPDATE SKIP LOCKED` SQL.
- Worker cancellation (stop via `worker_manager.stop_workers`) leaves no orphaned PEL entries — tasks are re-injected into the stream before the consumer exits.

---

## Potential Risks and Mitigations

1. **Redis unavailability causes total queue stall**
   Mitigation: Add Redis to health check (`/health` returns `503` on Redis failure). Consider a short-term fallback mode that logs an alarm and retries the Redis connection in a loop with exponential backoff, without falling back to the PostgreSQL queue path (which is being removed).

2. **Stream grows unboundedly if tasks are never ACKed**
   Mitigation: Add `MAXLEN ~10000` trim option to the `XADD` call in `enqueue_task`. Completed/failed messages are ACKed (removed from PEL) but remain in the stream log — use `XTRIM fetcher:tasks MAXLEN ~ 50000` in a periodic maintenance job or at bootstrap.

3. **Two-step enqueue (PG insert → XADD) partial failure**
   If the PG insert succeeds but `XADD` fails, the task row exists in `fetch_tasks` but never enters the stream.
   Mitigation: In `enqueue_task`, on `XADD` failure, set `fetch_tasks.status = 'failed'` with an error note, then return the error. On retry (same PMC+query), the `ON CONFLICT DO UPDATE` resets the row and retries `XADD`. Log the anomaly.

4. **`stream_message_id` NULL on tasks inserted before migration**
   Tasks in `fetch_tasks` with `stream_message_id IS NULL` (pre-migration) cannot be `XACK`ed.
   Mitigation: Write a one-time migration script that either marks pre-existing `in_progress` rows as `failed` (triggering re-enqueue by the caller) or sets their `status = 'pending'` and re-enqueues them via `XADD` as part of the data migration step, backfilling `stream_message_id`.

5. **Consumer group not existing at first startup**
   Mitigation: `bootstrap_redis_queue` (Task 6.1) runs at startup and uses `MKSTREAM` + tolerates `BUSYGROUP` error — idempotent across multiple replicas starting simultaneously.

6. **`XAUTOCLAIM` reclaim by wrong worker during worker pool resize**
   If a worker is gracefully stopping while another calls `XAUTOCLAIM`, a task could be double-claimed.
   Mitigation: The graceful stop path (Task 4.4) re-injects via `XADD` and immediately `XACK`s the PEL entry before the worker exits, leaving no orphaned PEL entries for `XAUTOCLAIM` to reclaim.

---

## Alternative Approaches

1. **Redis List + BRPOP (simpler, no consumer groups)**: Use `RPUSH` / `BLPOP` instead of streams. Simpler to implement, but lacks built-in PEL, `XAUTOCLAIM`, and message replay. No acknowledge mechanism — a crashed worker loses the task permanently. Rejected because the user specifically requested acknowledge-based tracking.

2. **Redis List + BRPOP + separate ACK set**: `BRPOP` for claiming, `SADD fetcher:ack-pending {task_id}` for in-progress tracking, `SREM` on completion. Provides a crude ack mechanism but requires manual stale-detection logic (ZSET with scores as timestamps) to replace `XAUTOCLAIM`. More code to maintain than streams.

3. **Fully drop `fetch_tasks` table**: Remove PostgreSQL queue table entirely; store all state in Redis hashes. Reduces write amplification but eliminates the rich SQL-based stats queries in `server.rs` (14+ aggregation queries). Re-implementing them against Redis would require SCAN + HGETALL loops, degrading stats performance. Rejected in favour of the hybrid approach (Redis for queue, PostgreSQL for state mirror and stats).

4. **Use a dedicated message broker (RabbitMQ, NATS JetStream)**: Provides durable queues, routing, dead-letter exchanges, etc. Operationally heavier (new service, new client library, ops knowledge). Redis is already available in the infrastructure (referenced in `FIXME.md` common section). Rejected for now but a valid future path if queue throughput demands grow significantly.
