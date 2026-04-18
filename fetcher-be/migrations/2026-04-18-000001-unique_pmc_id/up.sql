-- Change UNIQUE constraint from (pmc_id, query) to (pmc_id) only.
-- This ensures the same paper is never fetched twice regardless of which query discovered it.
-- The continuous pipeline re-runs all queries every cycle; this dedup prevents redundant work.

-- Step 1: Remove duplicate rows, keeping the earliest task per pmc_id
DELETE FROM fetch_task_components
WHERE task_id IN (
    SELECT id FROM fetch_tasks
    WHERE id NOT IN (
        SELECT MIN(id) FROM fetch_tasks GROUP BY pmc_id
    )
);

DELETE FROM fetch_task_logs
WHERE task_id IN (
    SELECT id FROM fetch_tasks
    WHERE id NOT IN (
        SELECT MIN(id) FROM fetch_tasks GROUP BY pmc_id
    )
);

DELETE FROM fetch_tasks
WHERE id NOT IN (
    SELECT MIN(id) FROM fetch_tasks GROUP BY pmc_id
);

-- Step 2: Drop old constraint, add new one
ALTER TABLE fetch_tasks DROP CONSTRAINT fetch_tasks_pmc_id_query_key;
ALTER TABLE fetch_tasks ADD CONSTRAINT fetch_tasks_pmc_id_key UNIQUE (pmc_id);
