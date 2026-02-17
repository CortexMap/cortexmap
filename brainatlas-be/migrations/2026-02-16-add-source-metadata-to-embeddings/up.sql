-- Add source metadata columns to brain_region_embeddings table
ALTER TABLE brain_region_embeddings
  ADD COLUMN source_pmc_id VARCHAR(20),
  ADD COLUMN source_uid VARCHAR(20),
  ADD COLUMN source_s3_key TEXT,
  ADD COLUMN source_query TEXT;

CREATE INDEX idx_embeddings_source_pmc ON brain_region_embeddings(source_pmc_id);

COMMENT ON COLUMN brain_region_embeddings.source_pmc_id IS 'PubMed Central ID (e.g., PMC12345) extracted from S3 key';
COMMENT ON COLUMN brain_region_embeddings.source_uid IS 'PubMed UID for citation and linking';
COMMENT ON COLUMN brain_region_embeddings.source_s3_key IS 'Original S3 key for the paper text file';
COMMENT ON COLUMN brain_region_embeddings.source_query IS 'PubMed query that retrieved this paper';
