-- Create fetch_tasks table for queue-based processing
CREATE TABLE fetch_tasks (
    id BIGSERIAL PRIMARY KEY,
    pmc_id TEXT NOT NULL,
    query TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'in_progress', 'completed', 'failed')),
    priority INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    started_at TIMESTAMP,
    completed_at TIMESTAMP,
    last_processed_at TIMESTAMP,
    UNIQUE(pmc_id, query)
);

-- Index for efficient queue polling (most important query)
CREATE INDEX idx_fetch_tasks_queue_polling ON fetch_tasks(status, priority DESC, created_at ASC);

-- Index for PMC ID lookups
CREATE INDEX idx_fetch_tasks_pmc_id ON fetch_tasks(pmc_id);

-- Index for monitoring queries
CREATE INDEX idx_fetch_tasks_status ON fetch_tasks(status);

-- Function to automatically update updated_at timestamp
CREATE OR REPLACE FUNCTION diesel_manage_updated_at(_tbl regclass) RETURNS VOID AS $$
BEGIN
    EXECUTE format('CREATE TRIGGER set_updated_at BEFORE UPDATE ON %s
                    FOR EACH ROW EXECUTE PROCEDURE diesel_set_updated_at()', _tbl);
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION diesel_set_updated_at() RETURNS trigger AS $$
BEGIN
    IF (
        NEW IS DISTINCT FROM OLD AND
        NEW.updated_at IS NOT DISTINCT FROM OLD.updated_at
    ) THEN
        NEW.updated_at := current_timestamp;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Apply the trigger to fetch_tasks
SELECT diesel_manage_updated_at('fetch_tasks');
