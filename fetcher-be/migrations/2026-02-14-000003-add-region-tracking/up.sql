-- Add region_id to fetch_tasks to link tasks to brain regions
ALTER TABLE fetch_tasks 
  ADD COLUMN region_id INTEGER;

-- Index for fast lookup of tasks by region and status
CREATE INDEX idx_fetch_tasks_region_status 
  ON fetch_tasks(region_id, status);

-- Comment for documentation
COMMENT ON COLUMN fetch_tasks.region_id IS 
  'Brain region ID from region_mapping table - set by orch when enqueuing tasks';
