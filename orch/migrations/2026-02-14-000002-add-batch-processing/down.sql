-- Remove batch processing tables and config
DELETE FROM orch_config WHERE key = 'query_generation_limit';
DROP TABLE IF EXISTS region_processing_batches;
DROP TABLE IF EXISTS region_queries;
