-- Restore unique constraints on region_summary

ALTER TABLE region_summary ADD CONSTRAINT region_summary_name_key UNIQUE (name);
ALTER TABLE region_summary ADD CONSTRAINT region_summary_region_id_key UNIQUE (region_id);
