-- Add is_active column to track which summary is currently valid/active
-- Only the most recent summary per region should be active
-- When invalidated, is_active = false, forcing generation of a new summary
ALTER TABLE region_summary 
  ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT true;

-- Create index for quick lookup of active summaries
CREATE INDEX idx_region_summary_active 
  ON region_summary(region_id, is_active) 
  WHERE is_active = true;

-- Set all existing summaries to active initially
-- (For each region, mark only the most recent as active)
WITH ranked_summaries AS (
  SELECT 
    id,
    ROW_NUMBER() OVER (PARTITION BY region_id ORDER BY created_at DESC) as rn
  FROM region_summary
)
UPDATE region_summary
SET is_active = (region_summary.id IN (
  SELECT id FROM ranked_summaries WHERE rn = 1
));

COMMENT ON COLUMN region_summary.is_active IS 
  'Whether this summary is currently active/valid. Only the most recent valid summary per region should be true. Invalidation sets this to false, forcing generation of a new summary.';
