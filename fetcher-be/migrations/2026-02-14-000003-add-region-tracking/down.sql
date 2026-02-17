-- Remove region tracking
DROP INDEX IF EXISTS idx_fetch_tasks_region_status;
ALTER TABLE fetch_tasks DROP COLUMN IF EXISTS region_id;
