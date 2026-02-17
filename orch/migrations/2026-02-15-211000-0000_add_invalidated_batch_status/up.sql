-- Add 'invalidated' status to region_processing_batches status check constraint

-- Drop the old constraint
ALTER TABLE region_processing_batches DROP CONSTRAINT status_check;

-- Add new constraint with 'invalidated' status
ALTER TABLE region_processing_batches 
ADD CONSTRAINT status_check 
CHECK (status IN ('collecting', 'ready', 'processing', 'completed', 'failed', 'invalidated'));

