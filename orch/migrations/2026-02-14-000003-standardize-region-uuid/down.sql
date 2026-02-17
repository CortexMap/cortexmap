-- Rollback: Change region_id back from UUID to Int4

-- Clear data
TRUNCATE TABLE region_processing_batches CASCADE;
TRUNCATE TABLE region_queries CASCADE;

-- Revert region_queries
ALTER TABLE region_queries
  DROP CONSTRAINT IF EXISTS fk_region_mapping,
  DROP COLUMN region_id,
  ADD COLUMN region_id INTEGER NOT NULL;

DROP INDEX IF EXISTS idx_region_queries_region;
CREATE INDEX idx_region_queries_region 
  ON region_queries(region_id) 
  WHERE enabled = true;

-- Revert region_processing_batches
ALTER TABLE region_processing_batches
  DROP CONSTRAINT IF EXISTS fk_region_mapping,
  DROP COLUMN region_id,
  ADD COLUMN region_id INTEGER NOT NULL;

DROP INDEX IF EXISTS idx_batches_region_status;
CREATE INDEX idx_batches_region_status 
  ON region_processing_batches(region_id, status);

DROP INDEX IF EXISTS idx_one_active_batch_per_region;
CREATE UNIQUE INDEX idx_one_active_batch_per_region 
  ON region_processing_batches(region_id) 
  WHERE status IN ('collecting', 'ready', 'processing');
