-- Drop embeddings table
DROP TABLE IF EXISTS brain_region_embeddings;

-- Remove hash column and index from region_summary
DROP INDEX IF EXISTS idx_region_summary_hash;
ALTER TABLE region_summary DROP COLUMN IF EXISTS content_hash;

-- Note: Not dropping vector extension (other tables might use it)
-- If you want to drop it: DROP EXTENSION IF EXISTS vector CASCADE;
