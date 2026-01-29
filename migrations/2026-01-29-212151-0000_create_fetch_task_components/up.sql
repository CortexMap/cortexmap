-- Create fetch_task_components table for granular component tracking
CREATE TABLE fetch_task_components (
    id BIGSERIAL PRIMARY KEY,
    task_id BIGINT NOT NULL REFERENCES fetch_tasks(id) ON DELETE CASCADE,
    component_type TEXT NOT NULL CHECK (component_type IN ('summary', 'abstract', 'pdf')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'in_progress', 'completed', 'failed')),
    s3_key TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    error_message TEXT,
    last_attempted_at TIMESTAMP,
    completed_at TIMESTAMP,
    UNIQUE(task_id, component_type)
);

-- Index for component queries by task
CREATE INDEX idx_fetch_task_components_task_id ON fetch_task_components(task_id, status);

-- Index for monitoring component status
CREATE INDEX idx_fetch_task_components_status ON fetch_task_components(status);

-- Index for component type queries
CREATE INDEX idx_fetch_task_components_type ON fetch_task_components(component_type);
