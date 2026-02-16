-- Remove is_active column from region_summary
DROP INDEX IF EXISTS idx_region_summary_active;
ALTER TABLE region_summary DROP COLUMN IF EXISTS is_active;
