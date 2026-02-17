-- Backfill source metadata for existing embeddings that have NULL source fields.
--
-- Strategy A: embeddings that already have source_s3_key set (new embeds from code changes).
-- Join with fetch_task_components + fetch_tasks to fill pmc_id and query from the S3 key.
UPDATE brain_region_embeddings bre
SET
    source_pmc_id = COALESCE(
        bre.source_pmc_id,
        (regexp_match(bre.source_s3_key, 'papers/(PMC[0-9]+)/'))[1]
    ),
    source_query = COALESCE(
        bre.source_query,
        ft.query
    )
FROM fetch_task_components ftc
JOIN fetch_tasks ft ON ft.id = ftc.task_id
WHERE bre.source_s3_key IS NOT NULL
  AND bre.source_s3_key = ftc.s3_key
  AND (bre.source_pmc_id IS NULL OR bre.source_query IS NULL);

-- Strategy B: embeddings with no source_s3_key (old embeddings before source tracking).
-- Recover s3_key, pmc_id, and query via:
--   brain_region_embeddings.summary_id
--     -> region_summary.batch_id
--       -> region_processing_batches.fetch_task_ids
--         -> fetch_task_components (s3_key)
--         -> fetch_tasks (pmc_id, query)
-- One representative text file per embedding (DISTINCT ON + ORDER ensures one row each).
WITH candidates AS (
    SELECT DISTINCT ON (bre.id)
        bre.id AS embedding_id,
        ftc.s3_key,
        (regexp_match(ftc.s3_key, 'papers/(PMC[0-9]+)/'))[1] AS pmc_id,
        ft.query
    FROM brain_region_embeddings bre
    JOIN region_summary rs ON rs.id = bre.summary_id
    JOIN region_processing_batches rpb ON rpb.id = rs.batch_id
    JOIN fetch_task_components ftc ON ftc.task_id = ANY(
        (SELECT ARRAY_AGG(t) FROM UNNEST(rpb.fetch_task_ids) AS t WHERE t IS NOT NULL)::bigint[]
    )
    JOIN fetch_tasks ft ON ft.id = ftc.task_id
    WHERE bre.source_s3_key IS NULL
      AND ftc.s3_key IS NOT NULL
      AND ftc.s3_key NOT LIKE '%.pdf'
    ORDER BY bre.id, ftc.s3_key
)
UPDATE brain_region_embeddings bre
SET
    source_s3_key = c.s3_key,
    source_pmc_id = c.pmc_id,
    source_query  = c.query
FROM candidates c
WHERE bre.id = c.embedding_id
  AND bre.source_s3_key IS NULL;
