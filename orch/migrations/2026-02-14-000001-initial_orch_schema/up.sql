-- Orch orchestration tables
-- Tracks which fetch tasks have been processed by brainatlas

CREATE TABLE processed_fetch_tasks (
    fetch_task_id BIGINT PRIMARY KEY,
    region_id UUID NOT NULL,
    pmc_id TEXT NOT NULL,
    processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    brainatlas_status TEXT NOT NULL DEFAULT 'pending',
    brainatlas_started_at TIMESTAMP,
    brainatlas_completed_at TIMESTAMP,
    error_message TEXT,
    CONSTRAINT brainatlas_status_check CHECK (brainatlas_status IN ('pending', 'in_progress', 'completed', 'failed'))
);

CREATE INDEX idx_processed_fetch_tasks_status ON processed_fetch_tasks(brainatlas_status);
CREATE INDEX idx_processed_fetch_tasks_region ON processed_fetch_tasks(region_id);
CREATE INDEX idx_processed_fetch_tasks_processed_at ON processed_fetch_tasks(processed_at DESC);

-- Orch configuration - all tuneable values
CREATE TABLE orch_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Insert default configuration
INSERT INTO orch_config (key, value, description) VALUES
    ('completion_poll_interval_secs', '30', 'How often to check for completed fetch tasks'),
    ('region_scan_interval_secs', '86400', 'How often to scan for stale region summaries (24 hours)'),
    ('max_parallel_process_calls', '10', 'Max concurrent calls to brainatlas /process'),
    ('summary_staleness_days', '30', 'Consider summaries older than N days stale'),
    ('fetcher_base_url', 'http://localhost:8080', 'Base URL for fetcher-be service'),
    ('brainatlas_base_url', 'http://localhost:8082', 'Base URL for brainatlas-be service');
