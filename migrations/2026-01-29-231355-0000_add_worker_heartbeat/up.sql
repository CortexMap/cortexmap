-- Add worker tracking and heartbeat columns to fetch_tasks
ALTER TABLE fetch_tasks
    ADD COLUMN worker_id TEXT,
    ADD COLUMN heartbeat_at TIMESTAMP,
    ADD COLUMN worker_version TEXT;

-- Index for finding tasks by worker (for cleanup when worker stops)
CREATE INDEX idx_fetch_tasks_worker_id ON fetch_tasks(worker_id) WHERE worker_id IS NOT NULL;

-- Index for finding stale tasks (old heartbeat)
CREATE INDEX idx_fetch_tasks_stale ON fetch_tasks(status, heartbeat_at) 
    WHERE status = 'in_progress' AND heartbeat_at IS NOT NULL;

-- Function to release tasks from a specific worker
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

-- Function to release stale tasks (heartbeat timeout)
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

COMMENT ON COLUMN fetch_tasks.worker_id IS 'Unique ID of the worker currently processing this task';
COMMENT ON COLUMN fetch_tasks.heartbeat_at IS 'Last heartbeat timestamp from the worker';
COMMENT ON COLUMN fetch_tasks.worker_version IS 'Version of the worker software';
COMMENT ON FUNCTION release_worker_tasks(TEXT) IS 'Release all tasks assigned to a specific worker';
COMMENT ON FUNCTION release_stale_tasks(INTEGER) IS 'Release tasks with stale heartbeats (worker likely crashed)';
