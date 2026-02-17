-- Remove character offset columns from brain_region_embeddings
ALTER TABLE brain_region_embeddings
  DROP COLUMN IF EXISTS source_char_start,
  DROP COLUMN IF EXISTS source_char_end;
