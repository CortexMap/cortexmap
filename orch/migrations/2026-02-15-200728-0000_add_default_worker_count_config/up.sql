-- Add default_worker_count and query_generation_limit to orch_config
-- These values may already exist in fresh installations, so we use INSERT ... ON CONFLICT

INSERT INTO orch_config (key, value, description) 
VALUES 
    ('default_worker_count', '2', 'Default number of workers to allocate when tasks are enqueued'),
    ('query_generation_limit', '3', 'Number of search queries to generate per brain region')
ON CONFLICT (key) DO UPDATE 
SET description = EXCLUDED.description;

