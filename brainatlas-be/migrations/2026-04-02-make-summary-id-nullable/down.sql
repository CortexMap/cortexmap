-- Revert: make summary_id NOT NULL again
-- First set any NULLs to a zero UUID so the constraint can be applied
UPDATE brain_region_embeddings SET summary_id = '00000000-0000-0000-0000-000000000000' WHERE summary_id IS NULL;
ALTER TABLE brain_region_embeddings ALTER COLUMN summary_id SET NOT NULL;
