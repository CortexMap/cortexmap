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
