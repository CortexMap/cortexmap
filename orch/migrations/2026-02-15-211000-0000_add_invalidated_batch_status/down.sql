-- Remove 'invalidated' status from region_processing_batches

-- Drop the constraint
ALTER TABLE region_processing_batches DROP CONSTRAINT status_check;

-- Add back old constraint without 'invalidated'
ALTER TABLE region_processing_batches 
ADD CONSTRAINT status_check 
CHECK (status IN ('collecting', 'ready', 'processing', 'completed', 'failed'));

