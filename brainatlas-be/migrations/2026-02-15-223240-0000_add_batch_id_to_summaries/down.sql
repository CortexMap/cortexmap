-- Revert the batch_id column addition
DROP INDEX IF EXISTS idx_region_summary_batch_id;
ALTER TABLE region_summary DROP COLUMN IF EXISTS batch_id;
