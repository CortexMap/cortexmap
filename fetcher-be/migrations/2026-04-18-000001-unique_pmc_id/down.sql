-- Revert: restore UNIQUE(pmc_id, query) constraint
ALTER TABLE fetch_tasks DROP CONSTRAINT IF EXISTS fetch_tasks_pmc_id_key;
ALTER TABLE fetch_tasks ADD CONSTRAINT fetch_tasks_pmc_id_query_key UNIQUE (pmc_id, query);
