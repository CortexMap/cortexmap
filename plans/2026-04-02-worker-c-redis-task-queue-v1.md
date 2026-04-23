# Worker C — RedisTaskQueue Implementation

## Objective

Replace `StdTaskQueue` with `RedisTaskQueue` in `crates/std-infra/src/task_queue.rs`. This worker exclusively owns this one file. `crates/std-infra/src/infra.rs` is owned by Worker A who will wire up the imports and delegation; Worker C only writes the struct and its `TaskQueueInfra` impl.

All paths are relative to `fetcher-be/`.

---

## Context / Assumed API (from Worker A)

Worker A will have established:
- `InfraError::RedisError(String)` variant in `cortexmap-infra/src/error.rs`
- `StdRedisInfra` in `crates/std-infra/src/redis_infra.rs` with:
  - `pub async fn get_conn(&self) -> Result<deadpool_redis::Connection, InfraError>`
  - `impl Clone for StdRedisInfra`
- `FetchTask` has a `pub stream_message_id: Option<String>` field
- `TaskQueueInfra` trait updated:
  - Removed: `reset_stale_tasks`, `release_worker_tasks`, `release_stale_tasks_by_heartbeat`
  - Updated: `get_next_pending_task(&self, timeout_secs: u64, worker_id: &str) -> Result<Option<FetchTask>, InfraError>`
  - Added: `reclaim_stale_tasks(&self, min_idle_ms: u64, worker_id: &str) -> Result<Vec<FetchTask>, InfraError>`
  - Added: `update_task_heartbeat_redis(&self, stream_id: &str, ttl_secs: u64) -> Result<(), InfraError>`

---

## Redis Key Conventions

| Key | Type | Purpose |
|---|---|---|
| `fetcher:tasks` | Stream | Primary task queue |
| `fetcher:workers` | Consumer Group name | On stream `fetcher:tasks` |
| `fetcher:task:{task_id}` | Hash | Live state mirror |
| `fetcher:heartbeat:{stream_id}` | String + EX | Worker liveness TTL |

Stream message fields (XADD): `task_id`, `pmc_id`, `query`, `priority`, `max_attempts`

---

## Implementation Plan

- [x] Task 1. **Define `RedisTaskQueue` struct**

  ```rust
  use crate::redis_infra::StdRedisInfra;
  use crate::database::DbPool;
  use diesel::r2d2::{ConnectionManager, Pool};
  use diesel::PgConnection;

  #[derive(Clone)]
  pub struct RedisTaskQueue {
      redis: StdRedisInfra,
      pg_pool: DbPool,
  }

  impl RedisTaskQueue {
      pub fn new(pg_pool: DbPool, redis: StdRedisInfra) -> Self {
          Self { redis, pg_pool }
      }

      // Copy the run_blocking helper verbatim from StdTaskQueue
      async fn run_blocking<F, T>(&self, f: F) -> Result<T, InfraError> { ... }
  }
  ```

  The `run_blocking` implementation is identical to the old `StdTaskQueue::run_blocking`.

- [x] Task 2. **`enqueue_task(pmc_id, query, max_attempts)`**

  Steps:
  1. Run the existing PG transaction (copy verbatim from `StdTaskQueue::enqueue_task` lines 54–95 of the current `task_queue.rs`) — this inserts the `FetchTask` row with `ON CONFLICT DO UPDATE SET updated_at = NOW()` and creates the three component rows with `ON CONFLICT DO NOTHING`.
  2. After PG returns the `FetchTask`:
     - If `task.stream_message_id.is_some()`, the task is already in the Redis stream (it was a duplicate enqueue). Return the task as-is.
     - Otherwise: call `XADD` and `HSET`, then update PG to set `stream_message_id`.
  3. `XADD` command (with `MAXLEN ~ 10000` to cap stream size):
     ```rust
     let stream_id: String = redis::cmd("XADD")
         .arg("MAXLEN").arg("~").arg(10000u64)
         .arg("fetcher:tasks")
         .arg("*")
         .arg("task_id").arg(task.id.to_string())
         .arg("pmc_id").arg(&task.pmc_id)
         .arg("query").arg(&task.query)
         .arg("priority").arg(task.priority.to_string())
         .arg("max_attempts").arg(max_attempts.to_string())
         .query_async(&mut conn).await
         .map_err(|e| InfraError::RedisError(e.to_string()))?;
     ```
  4. `HSET` to create the task state hash:
     ```rust
     redis::cmd("HSET")
         .arg(format!("fetcher:task:{}", task.id))
         .arg("stream_id").arg(&stream_id)
         .arg("status").arg("pending")
         .arg("pmc_id").arg(&task.pmc_id)
         .query_async::<i64>(&mut conn).await
         .map_err(|e| InfraError::RedisError(e.to_string()))?;
     ```
  5. Update PG `stream_message_id` column using `run_blocking`:
     ```sql
     UPDATE fetch_tasks SET stream_message_id = $1 WHERE id = $2
     ```
     via `diesel::sql_query(...)`.
  6. Return the task with `stream_message_id` set to `Some(stream_id)`.

- [x] Task 3. **`get_next_pending_task(timeout_secs: u64, worker_id: &str)`**

  1. Get Redis connection.
  2. Call `XREADGROUP GROUP fetcher:workers {worker_id} COUNT 1 BLOCK {timeout_ms} STREAMS fetcher:tasks >`:
     ```rust
     let block_ms = timeout_secs * 1000;
     let reply: redis::Value = redis::cmd("XREADGROUP")
         .arg("GROUP").arg("fetcher:workers").arg(worker_id)
         .arg("COUNT").arg(1)
         .arg("BLOCK").arg(block_ms)
         .arg("STREAMS").arg("fetcher:tasks").arg(">")
         .query_async(&mut conn).await
         .map_err(|e| InfraError::RedisError(e.to_string()))?;
     ```
  3. Parse the reply. `XREADGROUP` returns `redis::Value::Nil` on timeout or `redis::Value::Array(...)`. The structure is: `Array([Array([BulkString("fetcher:tasks"), Array([Array([BulkString(msg_id), Array([BulkString(field), BulkString(val), ...])])])])])`.

     Parse out `msg_id` (the stream message ID) and `task_id` (from the fields). If nil or empty, return `Ok(None)`.

  4. Use `run_blocking` to load the full `FetchTask` by task_id from PG using `fetch_tasks::table.find(task_id).first(conn).optional()`.
  5. On `None` from PG (task deleted?): call `XACK` to clean up the orphaned PEL entry, return `Ok(None)`.
  6. Set `task.stream_message_id = Some(stream_id)` on the returned task (Diesel won't set it from XREADGROUP, so override it here).
  7. Return `Ok(Some(task))`.

  **Helper**: Write a private `fn parse_xreadgroup_reply(reply: redis::Value) -> Option<(String, i64)>` that extracts `(stream_id, task_id)`. Use recursive `redis::Value` matching. Return `None` on any unexpected structure.

- [x] Task 4. **`claim_task_for_worker(task_id, worker_id, worker_version)`**

  Two operations:
  1. `HSET fetcher:task:{task_id} status in_progress worker_id {worker_id} worker_version {version} started_at {unix_now}` via `redis::cmd("HSET")`.
  2. PG `UPDATE fetch_tasks SET worker_id=?, worker_version=?, status='in_progress', started_at=NOW(), heartbeat_at=NOW() WHERE id=?` via `run_blocking` (identical to existing `StdTaskQueue::claim_task_for_worker`).

- [x] Task 5. **`update_task_heartbeat(task_id)`**

  PG-only — copy verbatim from `StdTaskQueue::update_task_heartbeat`. Updates `heartbeat_at = NOW()` in `fetch_tasks`.

- [x] Task 6. **`update_task_heartbeat_redis(stream_id, ttl_secs)`**

  Redis `SET` with expiry:
  ```rust
  redis::cmd("SET")
      .arg(format!("fetcher:heartbeat:{}", stream_id))
      .arg(1u8)
      .arg("EX").arg(ttl_secs)
      .query_async::<redis::Value>(&mut conn).await
      .map_err(|e| InfraError::RedisError(e.to_string()))?;
  ```

- [x] Task 7. **`mark_task_completed(task_id)`**

  The caller holds a `FetchTask` with `stream_message_id`. The method signature is `mark_task_completed(&self, task_id: i64)` — it does not receive the stream_id. Resolve this: look up `stream_message_id` via `HGET fetcher:task:{task_id} stream_id` from Redis (fast, no PG round-trip).

  Steps:
  1. Get conn, `HGET fetcher:task:{task_id} stream_id` → `stream_id: String`.
  2. If stream_id is non-empty: `XACK fetcher:tasks fetcher:workers {stream_id}`.
  3. `HSET fetcher:task:{task_id} status completed completed_at {unix_now}`.
  4. `EXPIRE fetcher:task:{task_id} 604800` (7 days).
  5. `DEL fetcher:heartbeat:{stream_id}` (if stream_id non-empty).
  6. PG UPDATE: copy verbatim from `StdTaskQueue::mark_task_completed`.

- [x] Task 8. **`mark_task_failed(task_id, error)`**

  Steps:
  1. `HGET fetcher:task:{task_id} stream_id` → `stream_id`.
  2. If non-empty: `XACK fetcher:tasks fetcher:workers {stream_id}`.
  3. `HSET fetcher:task:{task_id} status failed error {error_msg}` + `EXPIRE 604800`.
  4. `DEL fetcher:heartbeat:{stream_id}`.
  5. `log_task_event` call + PG UPDATE — copy verbatim from `StdTaskQueue::mark_task_failed`.

- [x] Task 9. **`release_task(task_id)`**

  Release and immediately re-enqueue (for partial failures — task goes back to front of queue):
  1. `HGET fetcher:task:{task_id} stream_id` → old `stream_id`.
  2. Re-inject via `XADD`:
     - Get pmc_id, query, priority from `HGETALL fetcher:task:{task_id}` or from a PG fetch.
     - `XADD fetcher:tasks MAXLEN ~ 10000 * task_id {id} pmc_id {pmc_id} query {query} priority {priority} max_attempts {attempts}` → `new_stream_id`.
  3. Update `HSET fetcher:task:{task_id} stream_id {new_stream_id} status pending worker_id "" heartbeat_at ""`.
  4. If old `stream_id` is non-empty: `XACK fetcher:tasks fetcher:workers {old_stream_id}`.
  5. `DEL fetcher:heartbeat:{old_stream_id}`.
  6. Update PG via `run_blocking`: `UPDATE fetch_tasks SET status='pending', worker_id=NULL, heartbeat_at=NULL, started_at=NULL, stream_message_id=$2 WHERE id=$1` using `diesel::sql_query`.

- [x] Task 10. **`reclaim_stale_tasks(min_idle_ms, worker_id)`**

  Uses `XAUTOCLAIM` to take over PEL entries idle for >= `min_idle_ms`:
  ```rust
  let reply: redis::Value = redis::cmd("XAUTOCLAIM")
      .arg("fetcher:tasks")
      .arg("fetcher:workers")
      .arg(worker_id)
      .arg(min_idle_ms)
      .arg("0-0")
      .arg("COUNT").arg(50u64)
      .query_async(&mut conn).await
      .map_err(|e| InfraError::RedisError(e.to_string()))?;
  ```
  Parse the response — `XAUTOCLAIM` returns `[next_id, [[msg_id, [fields...]], ...], [deleted_ids...]]`.
  For each reclaimed message:
  1. Parse `task_id` from fields.
  2. Update `HSET fetcher:task:{task_id} worker_id {worker_id} heartbeat_at {unix_now} status in_progress`.
  3. Load `FetchTask` from PG and set `stream_message_id = Some(msg_id)`.
  4. Update PG: `UPDATE fetch_tasks SET worker_id=$1, heartbeat_at=NOW(), status='in_progress', stream_message_id=$2 WHERE id=$3`.

  Return the vector of reclaimed `FetchTask`s. On an empty reclaim (stream doesn't exist yet or no stale entries), return `Ok(vec![])`.

- [x] Task 11. **`get_task_stats()`**

  Augment the PG-only counts with Redis data:
  - For `pending` and `in_progress`: call `self.redis.queue_pending_and_pel_count().await` to get `(total_stream_len, pel_count)`. Set `pending = total_stream_len - pel_count`, `in_progress = pel_count`.
  - For `completed` and `failed`: use the existing PG COUNT queries (copy from `StdTaskQueue`).
  - If Redis call fails, fall back to PG counts for pending/in_progress (log a warning).

- [x] Task 12. **`get_detailed_task_stats()`**

  Copy verbatim from `StdTaskQueue::get_detailed_task_stats` **except** replace the PG-derived `basic: TaskStats` with the Redis-augmented `self.get_task_stats().await?` result.

- [x] Task 13. **All remaining `TaskQueueInfra` methods — verbatim copy**

  Copy these methods exactly from `StdTaskQueue` (only change `StdTaskQueue` → `RedisTaskQueue` struct name internally):
  - `mark_task_started`
  - `get_pending_components`
  - `update_component_status`
  - `increment_component_attempt`
  - `all_components_completed`
  - `log_task_event`
  - `get_component_stats`
  - `get_recent_tasks`
  - `get_task_by_pmc_id`
  - `get_task_by_id`
  - `get_tasks_by_status`
  - `get_task_components`

---

## Verification Criteria

- File compiles without errors when Worker A's changes are also present.
- `RedisTaskQueue` implements all methods in the updated `TaskQueueInfra` trait (no missing methods).
- `enqueue_task` issues both a PG insert and `XADD`.
- `get_next_pending_task` uses `XREADGROUP` not `FOR UPDATE SKIP LOCKED`.
- `mark_task_completed` issues `XACK`.
- `release_task` re-injects via `XADD` and ACKs the old PEL entry.
- `reclaim_stale_tasks` uses `XAUTOCLAIM`.
- `get_task_stats` uses Redis PEL count for `in_progress`.
