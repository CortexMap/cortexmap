-- Add batch_id column to region_summary to track which orch batch generated each summary
ALTER TABLE region_summary 
ADD COLUMN batch_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000';

-- Remove the default after adding the column (for future inserts to require explicit batch_id)
ALTER TABLE region_summary ALTER COLUMN batch_id DROP DEFAULT;

-- Add index for querying summaries by batch
CREATE INDEX idx_region_summary_batch_id ON region_summary(batch_id);

-- Add comment explaining the column
COMMENT ON COLUMN region_summary.batch_id IS 'The orch processing batch ID that generated this summary. Uses 00000000-0000-0000-0000-000000000000 for summaries created before batch tracking was added.';
