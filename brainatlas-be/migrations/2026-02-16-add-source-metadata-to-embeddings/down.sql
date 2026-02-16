-- Remove source metadata columns from brain_region_embeddings table
DROP INDEX IF EXISTS idx_embeddings_source_pmc;

ALTER TABLE brain_region_embeddings
  DROP COLUMN source_pmc_id,
  DROP COLUMN source_uid,
  DROP COLUMN source_s3_key,
  DROP COLUMN source_query;
