-- Change UNIQUE constraint from (pmc_id, query) to (pmc_id) only.
-- Each paper is fetched exactly once regardless of which query discovered it.
-- The continuous pipeline re-runs all queries every cycle; this dedup prevents
-- workers from re-downloading the same PDF/abstract/summary N times.

-- Step 1: Pick the survivor per pmc_id.
-- Prefer completed > in_progress > pending > failed, then lowest id as tiebreaker.
CREATE TEMP TABLE _dedup_survivors AS
SELECT DISTINCT ON (pmc_id) id, pmc_id
FROM fetch_tasks
ORDER BY pmc_id,
         CASE status
             WHEN 'completed'   THEN 0
             WHEN 'in_progress' THEN 1
             WHEN 'pending'     THEN 2
             ELSE 3
         END,
         id ASC;

-- Step 2: Build old_id → new_id mapping for deleted rows.
CREATE TEMP TABLE _dedup_mapping AS
SELECT ft.id AS old_id, s.id AS new_id
FROM fetch_tasks ft
JOIN _dedup_survivors s USING (pmc_id)
WHERE ft.id != s.id;

-- Step 3: Remap batch fetch_task_ids arrays so they point at survivors.
-- Uses unnest → left-join → re-aggregate to replace each deleted id with its survivor.
UPDATE region_processing_batches rpb
SET fetch_task_ids = remapped.new_ids
FROM (
    SELECT rpb2.id AS batch_id,
           array_agg(DISTINCT COALESCE(dm.new_id, elem) ORDER BY COALESCE(dm.new_id, elem)) AS new_ids
    FROM region_processing_batches rpb2,
         unnest(rpb2.fetch_task_ids) AS elem
    LEFT JOIN _dedup_mapping dm ON dm.old_id = elem
    GROUP BY rpb2.id
) remapped
WHERE rpb.id = remapped.batch_id
  AND rpb.fetch_task_ids IS NOT NULL;

-- Step 4: Delete orphaned children, then duplicate tasks.
DELETE FROM fetch_task_components
WHERE task_id IN (SELECT old_id FROM _dedup_mapping);

DELETE FROM fetch_task_logs
WHERE task_id IN (SELECT old_id FROM _dedup_mapping);

DELETE FROM fetch_tasks
WHERE id IN (SELECT old_id FROM _dedup_mapping);

-- Step 5: Swap the constraint.
ALTER TABLE fetch_tasks DROP CONSTRAINT fetch_tasks_pmc_id_query_key;
ALTER TABLE fetch_tasks ADD CONSTRAINT fetch_tasks_pmc_id_key UNIQUE (pmc_id);

-- Cleanup
DROP TABLE _dedup_mapping;
DROP TABLE _dedup_survivors;
