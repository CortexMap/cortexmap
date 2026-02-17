-- Remove default_worker_count and query_generation_limit from orch_config
DELETE FROM orch_config WHERE key IN ('default_worker_count', 'query_generation_limit');

