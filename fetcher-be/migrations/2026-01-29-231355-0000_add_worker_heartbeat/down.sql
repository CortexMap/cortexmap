-- Drop functions
DROP FUNCTION IF EXISTS release_stale_tasks(INTEGER);
DROP FUNCTION IF EXISTS release_worker_tasks(TEXT);

-- Drop indexes
DROP INDEX IF EXISTS idx_fetch_tasks_stale;
DROP INDEX IF EXISTS idx_fetch_tasks_worker_id;

-- Drop columns
ALTER TABLE fetch_tasks
    DROP COLUMN IF EXISTS worker_version,
    DROP COLUMN IF EXISTS heartbeat_at,
    DROP COLUMN IF EXISTS worker_id;
