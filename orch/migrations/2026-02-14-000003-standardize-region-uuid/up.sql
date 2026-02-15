-- Standardize region_id to UUID across all orch tables
-- This makes the system consistent with region_mapping.id (UUID)

-- First, clear any existing data (tables are new, likely empty)
TRUNCATE TABLE region_processing_batches CASCADE;
TRUNCATE TABLE region_queries CASCADE;

-- Change region_id from Int4 to UUID in region_queries
ALTER TABLE region_queries 
  DROP COLUMN region_id,
  ADD COLUMN region_id UUID NOT NULL;

-- Add foreign key to region_mapping.id
ALTER TABLE region_queries
  ADD CONSTRAINT fk_region_mapping
  FOREIGN KEY (region_id)
  REFERENCES region_mapping(id)
  ON DELETE CASCADE;

-- Recreate index on new UUID column
DROP INDEX IF EXISTS idx_region_queries_region;
CREATE INDEX idx_region_queries_region 
  ON region_queries(region_id) 
  WHERE enabled = true;

-- Change region_id from Int4 to UUID in region_processing_batches
ALTER TABLE region_processing_batches
  DROP COLUMN region_id,
  ADD COLUMN region_id UUID NOT NULL;

-- Add foreign key to region_mapping.id
ALTER TABLE region_processing_batches
  ADD CONSTRAINT fk_region_mapping
  FOREIGN KEY (region_id)
  REFERENCES region_mapping(id)
  ON DELETE CASCADE;

-- Recreate indexes
DROP INDEX IF EXISTS idx_batches_region_status;
CREATE INDEX idx_batches_region_status 
  ON region_processing_batches(region_id, status);

DROP INDEX IF EXISTS idx_one_active_batch_per_region;
CREATE UNIQUE INDEX idx_one_active_batch_per_region 
  ON region_processing_batches(region_id) 
  WHERE status IN ('collecting', 'ready', 'processing');

COMMENT ON COLUMN region_queries.region_id IS 
  'UUID reference to region_mapping.id';

COMMENT ON COLUMN region_processing_batches.region_id IS 
  'UUID reference to region_mapping.id';
