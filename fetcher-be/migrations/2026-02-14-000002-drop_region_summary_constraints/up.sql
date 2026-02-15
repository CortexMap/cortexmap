-- Drop unique constraints on region_summary to allow multiple summaries per region
-- This enables storing multiple time-stamped summaries as papers are updated

ALTER TABLE region_summary DROP CONSTRAINT IF EXISTS region_summary_name_key;
ALTER TABLE region_summary DROP CONSTRAINT IF EXISTS region_summary_region_id_key;
