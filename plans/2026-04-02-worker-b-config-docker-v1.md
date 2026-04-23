# Worker B — Config & Docker

## Objective

Independent changes to config structs, docker-compose files, test setup, and a new DB migration to drop obsolete PG stored functions. All files in this plan are exclusively owned by this worker — no conflicts with other workers.

All paths are relative to `fetcher-be/` unless prefixed with the repo root.

---

## Implementation Plan

- [x] Task 1. **`crates/cortexmap-core/src/blueprint/connections/fetcher.rs` — update `RetryConfig`**
  In the `RetryConfig` struct (currently lines 24–42):
  1. **Remove** `stale_task_multiplier: u64` field and its doc comment.
  2. **Add** three new fields with doc comments:
     ```rust
     /// How often (seconds) a worker refreshes its heartbeat key in Redis.
     /// Default: 15
     pub heartbeat_interval_secs: u64,

     /// TTL (seconds) for the Redis heartbeat key. Should be > 2× heartbeat_interval_secs.
     /// Default: 45
     pub heartbeat_ttl_secs: u64,

     /// Minimum idle time (ms) before XAUTOCLAIM reclaims a PEL entry.
     /// Default: 60000 (60 seconds)
     pub stale_reclaim_min_idle_ms: u64,
     ```
  3. Update `impl Default for RetryConfig` (currently lines 89–98):
     - Remove `stale_task_multiplier: 10,`
     - Add:
       ```rust
       heartbeat_interval_secs: 15,
       heartbeat_ttl_secs: 45,
       stale_reclaim_min_idle_ms: 60_000,
       ```

- [x] Task 2. **New migration `migrations/2026-04-02-000002_drop_pg_queue_functions/`**
  Create both files under `fetcher-be/migrations/`:

  `up.sql` — drops the two stored functions that are replaced by Redis:
  ```sql
  DROP FUNCTION IF EXISTS release_worker_tasks(TEXT);
  DROP FUNCTION IF EXISTS release_stale_tasks(INTEGER);
  ```

  `down.sql` — recreates them (copy verbatim from `migrations/2026-01-29-231355-0000_add_worker_heartbeat/up.sql` lines 14–49, i.e. the two `CREATE OR REPLACE FUNCTION` blocks):
  ```sql
  CREATE OR REPLACE FUNCTION release_worker_tasks(p_worker_id TEXT) RETURNS INTEGER AS $$
  DECLARE
      affected_count INTEGER;
  BEGIN
      UPDATE fetch_tasks
      SET status = 'pending',
          worker_id = NULL,
          heartbeat_at = NULL,
          updated_at = NOW()
      WHERE worker_id = p_worker_id
        AND status = 'in_progress';
      GET DIAGNOSTICS affected_count = ROW_COUNT;
      RETURN affected_count;
  END;
  $$ LANGUAGE plpgsql;

  CREATE OR REPLACE FUNCTION release_stale_tasks(p_timeout_seconds INTEGER) RETURNS INTEGER AS $$
  DECLARE
      affected_count INTEGER;
  BEGIN
      UPDATE fetch_tasks
      SET status = 'pending',
          worker_id = NULL,
          heartbeat_at = NULL,
          updated_at = NOW()
      WHERE status = 'in_progress'
        AND heartbeat_at IS NOT NULL
        AND heartbeat_at < NOW() - (p_timeout_seconds || ' seconds')::INTERVAL;
      GET DIAGNOSTICS affected_count = ROW_COUNT;
      RETURN affected_count;
  END;
  $$ LANGUAGE plpgsql;
  ```

- [x] Task 3. **`docker-compose.app.yml` — add `REDIS_URL` to `cortexmap-be` and `depends_on`**
  The file already has a `redis` service defined. Two changes needed in the `cortexmap-be` service:
  1. Add to its `environment` block:
     ```yaml
     REDIS_URL: redis://redis:6379
     ```
  2. Add a `depends_on` block (it currently has none):
     ```yaml
     depends_on:
       redis:
         condition: service_healthy
     ```

- [x] Task 4. **`docker-compose.test.yml` — add `REDIS_URL` to test environment comment**
  The file already has `redis-test` on port `6380`. Add a comment block noting the test Redis URL for documentation purposes, and add a `REDIS_URL` environment variable to the postgres-test service notes (no service change needed — the test env var `REDIS_URL=redis://localhost:6380` is set by `test.sh`).

  Actually: inspect `test.sh` and `setup-test-data.sh` at repo root. In `test.sh`, find where `DATABASE_URL` is exported for integration tests, and add alongside it:
  ```bash
  export REDIS_URL="redis://localhost:6380"
  ```
  If `test.sh` does not exist or does not export env vars this way, check `setup-test-data.sh` for the same pattern and add the export there.

  Also add a `FLUSHDB` call in the test setup (after Redis is healthy) to ensure a clean state:
  ```bash
  redis-cli -h localhost -p 6380 FLUSHDB || true
  ```

---

## Verification Criteria

- `cargo check -p cortexmap-core` compiles without errors after RetryConfig changes.
- `RetryConfig::default()` has `heartbeat_interval_secs: 15`, `heartbeat_ttl_secs: 45`, `stale_reclaim_min_idle_ms: 60_000`.
- `stale_task_multiplier` no longer exists on `RetryConfig`.
- `docker-compose.app.yml` has `REDIS_URL: redis://redis:6379` under `cortexmap-be.environment`.
- Migration SQL files are valid and symmetric (up/down).
