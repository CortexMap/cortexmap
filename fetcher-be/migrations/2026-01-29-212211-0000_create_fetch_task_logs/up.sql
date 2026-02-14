-- Create fetch_task_logs table for audit trail and debugging
CREATE TABLE fetch_task_logs (
    id BIGSERIAL PRIMARY KEY,
    task_id BIGINT NOT NULL REFERENCES fetch_tasks(id) ON DELETE CASCADE,
    component_type TEXT,
    log_level TEXT NOT NULL CHECK (log_level IN ('debug', 'info', 'warn', 'error')),
    message TEXT NOT NULL,
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Index for retrieving logs by task in chronological order
CREATE INDEX idx_fetch_task_logs_task_id ON fetch_task_logs(task_id, created_at DESC);

-- Index for filtering by log level
CREATE INDEX idx_fetch_task_logs_level ON fetch_task_logs(log_level);

-- Index for time-based queries
CREATE INDEX idx_fetch_task_logs_created_at ON fetch_task_logs(created_at DESC);
