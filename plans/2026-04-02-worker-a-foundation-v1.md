# Worker A — Foundation Layer

## Objective

Build the Redis infrastructure foundation that all other workers depend on. Owns: workspace `Cargo.toml`, `std-infra` Cargo, `cortexmap-infra` error/trait/model/schema, new `redis_infra.rs`, updated `std-infra/src/lib.rs` + `infra.rs`, DB migration for `stream_message_id`. No other worker touches these files.

All paths are relative to `fetcher-be/`.

---

## Implementation Plan

- [x] Task 1. **`Cargo.toml` (workspace root) — add Redis deps**
  In `[workspace.dependencies]` add:
  ```toml
  redis = { version = "0.27", features = ["tokio-comp", "streams"] }
  deadpool-redis = { version = "0.18", features = ["rt_tokio_1"] }
  sha2 = "0.10"
  ```

- [x] Task 2. **`crates/std-infra/Cargo.toml` — inherit new deps**
  Add to `[dependencies]`:
  ```toml
  redis.workspace = true
  deadpool-redis.workspace = true
  sha2.workspace = true
  ```

- [x] Task 3. **`crates/cortexmap-infra/src/error.rs` — add `RedisError` variant**
  Add after the `EnvVarNotFound` variant:
  ```rust
  #[error("Redis error: {0}")]
  RedisError(String),
  ```

- [x] Task 4. **New file `crates/std-infra/src/redis_infra.rs`**
  Create this module with:

  ```rust
  pub type RedisPool = deadpool_redis::Pool;

  #[derive(Clone)]
  pub struct StdRedisInfra {
      pool: RedisPool,
  }
  ```

  `impl StdRedisInfra`:
  - `pub fn new(redis_url: &str) -> Result<Self, InfraError>`:
    Build `deadpool_redis::Config::from_url(redis_url)`, set `pool_config.max_size = 20`, call `.create_pool(Some(deadpool_redis::Runtime::Tokio1))`. Map `CreatePoolError` to `InfraError::RedisError(e.to_string())`.
  - `pub async fn get_conn(&self) -> Result<deadpool_redis::Connection, InfraError>`:
    `self.pool.get().await.map_err(|e| InfraError::RedisError(e.to_string()))`
  - `pub async fn bootstrap_queue(&self) -> Result<(), InfraError>`:
    Get conn, run `redis::cmd("XGROUP").arg("CREATE").arg("fetcher:tasks").arg("fetcher:workers").arg("$").arg("MKSTREAM").query_async::<String>(&mut conn).await`. If the error string contains `"BUSYGROUP"` treat as `Ok(())`. Map other errors to `InfraError::RedisError`.
  - `pub async fn ping(&self) -> Result<(), InfraError>`:
    Get conn, run `redis::cmd("PING").query_async::<String>(&mut conn).await`. Map error to `InfraError::RedisError`.
  - `pub async fn queue_pending_and_pel_count(&self) -> Result<(i64, i64), InfraError>`:
    Get conn. Run `redis::cmd("XLEN").arg("fetcher:tasks").query_async::<i64>(&mut conn).await` for total length. Run `redis::cmd("XINFO").arg("GROUPS").arg("fetcher:tasks").query_async::<redis::Value>(&mut conn).await` and parse the `pel-count` field for the `fetcher:workers` group from the returned `Value::Array`. The `XINFO GROUPS` response is a flat array of key-value pairs repeated per group; find the entry where the group name equals `"fetcher:workers"` and extract the value after the `"pel-count"` key. Return `(xlen, pel_count)`. On parse error return `(0, 0)`.

- [x] Task 5. **`crates/std-infra/src/lib.rs` — wire Redis into context**
  - Add `pub mod redis_infra;` (alongside existing mods).
  - Add `pub use redis_infra::StdRedisInfra;` to re-exports.
  - Add `redis_url: String` field to `StdInfraContext`.
  - In `StdInfraContext::from_env()`, add: `redis_url: env.get_env_var("REDIS_URL")?,`
  - In `StdInfraContext::get()`, pass `&self.redis_url` to `StdInfra::new(...)` (see Task 6).

- [x] Task 6. **`crates/std-infra/src/infra.rs` — full replacement**
  Rewrite this file. Key changes vs current:
  1. Replace `use crate::task_queue::StdTaskQueue;` with `use crate::task_queue::RedisTaskQueue;` (Worker C will provide this struct).
  2. Add `use crate::redis_infra::StdRedisInfra;`.
  3. In `StdInfra` struct: replace `task_queue: StdTaskQueue` with `task_queue: RedisTaskQueue`, add `redis: StdRedisInfra`.
  4. `StdInfra::new(database_url, endpoint, access_key, secret_key, bucket, redis_url: &str)`:
     - Build `db_infra` as before.
     - `let redis = StdRedisInfra::new(redis_url)?;`
     - `let task_queue = RedisTaskQueue::new(db_infra.pool.clone(), redis.clone());`
  5. Add `pub fn redis(&self) -> &StdRedisInfra { &self.redis }` accessor.
  6. Keep all existing trait impls (`EnvInfra`, `HttpInfra`, `DatabaseInfra`, `S3Infra`) verbatim.
  7. In `impl TaskQueueInfra for StdInfra`:
     - **Remove** the three delegation methods: `reset_stale_tasks`, `release_worker_tasks`, `release_stale_tasks_by_heartbeat`.
     - **Update** `get_next_pending_task` delegation to pass the new `worker_id` parameter: `self.task_queue.get_next_pending_task(timeout_secs, worker_id).await`
     - **Add** two new delegations:
       ```rust
       async fn reclaim_stale_tasks(&self, min_idle_ms: u64, worker_id: &str) -> Result<Vec<FetchTask>, InfraError> {
           self.task_queue.reclaim_stale_tasks(min_idle_ms, worker_id).await
       }
       async fn update_task_heartbeat_redis(&self, stream_id: &str, ttl_secs: u64) -> Result<(), InfraError> {
           self.task_queue.update_task_heartbeat_redis(stream_id, ttl_secs).await
       }
       ```
     - All other delegations are unchanged.

- [x] Task 7. **`crates/cortexmap-infra/src/infra.rs` — update `TaskQueueInfra` trait**
  The trait lives at lines 113–226. Make these changes:
  1. **Remove** three methods from the trait definition:
     - `async fn reset_stale_tasks(&self, timeout_secs: u64) -> Result<usize, InfraError>;`
     - `async fn release_worker_tasks(&self, worker_id: String) -> Result<usize, InfraError>;`
     - `async fn release_stale_tasks_by_heartbeat(&self, timeout_secs: u64) -> Result<usize, InfraError>;`
  2. **Update** `get_next_pending_task` signature — add `worker_id: &str` parameter:
     ```rust
     async fn get_next_pending_task(
         &self,
         timeout_secs: u64,
         worker_id: &str,
     ) -> Result<Option<FetchTask>, InfraError>;
     ```
  3. **Add** two new methods after `update_task_heartbeat`:
     ```rust
     /// Reclaim tasks whose heartbeat has been idle for >= min_idle_ms (XAUTOCLAIM)
     async fn reclaim_stale_tasks(
         &self,
         min_idle_ms: u64,
         worker_id: &str,
     ) -> Result<Vec<FetchTask>, InfraError>;

     /// Refresh the Redis heartbeat TTL key for an in-progress task
     async fn update_task_heartbeat_redis(
         &self,
         stream_id: &str,
         ttl_secs: u64,
     ) -> Result<(), InfraError>;
     ```
  4. Update the doc comment on `get_next_pending_task` — remove the mention of `FOR UPDATE SKIP LOCKED`, replace with `XREADGROUP`.

- [x] Task 8. **`crates/cortexmap-infra/src/database/models.rs` — add `stream_message_id` to `FetchTask`**
  Add as the last field of `FetchTask` (after `worker_version`):
  ```rust
  pub stream_message_id: Option<String>,
  ```
  The `#[diesel(table_name = fetch_tasks)]` and `#[diesel(check_for_backend(diesel::pg::Pg))]` derive attributes remain on the struct. Do not add this field to `NewFetchTask` (it is set after insert, not on creation).

- [x] Task 9. **`crates/cortexmap-infra/src/database/schema.rs` — add `stream_message_id` column**
  Inside the `fetch_tasks` table definition, add after `worker_version -> Nullable<Text>`:
  ```rust
  stream_message_id -> Nullable<Text>,
  ```

- [x] Task 10. **New migration `migrations/2026-04-02-000001_add_stream_message_id/`**
  Create both files:

  `up.sql`:
  ```sql
  ALTER TABLE fetch_tasks ADD COLUMN stream_message_id TEXT;
  CREATE INDEX idx_fetch_tasks_stream_id ON fetch_tasks(stream_message_id)
    WHERE stream_message_id IS NOT NULL;
  ```

  `down.sql`:
  ```sql
  DROP INDEX IF EXISTS idx_fetch_tasks_stream_id;
  ALTER TABLE fetch_tasks DROP COLUMN IF EXISTS stream_message_id;
  ```

---

## Verification Criteria

- `cargo check -p std-infra` compiles without errors (even if `task_queue.rs` is not yet updated — `RedisTaskQueue` is referenced but not yet defined; Forge may need to add a stub or the build will error there, which is acceptable).
- `cargo check -p cortexmap-infra` compiles without errors.
- `StdRedisInfra::new`, `bootstrap_queue`, `ping`, `queue_pending_and_pel_count` are all `pub`.
- `StdInfraContext::from_env()` reads `REDIS_URL` from env.
- Migration SQL files are valid PostgreSQL.
