-- Add character offset columns to brain_region_embeddings for source range tracking
ALTER TABLE brain_region_embeddings
  ADD COLUMN source_char_start INTEGER,
  ADD COLUMN source_char_end INTEGER;

COMMENT ON COLUMN brain_region_embeddings.source_char_start IS 'Character offset of the start of this chunk within the source S3 file';
COMMENT ON COLUMN brain_region_embeddings.source_char_end IS 'Character offset of the end of this chunk within the source S3 file';
