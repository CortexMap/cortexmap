INSERT INTO orch_config (key, value, description, updated_at)
VALUES ('search_result_limit', '5', 'Maximum number of results returned by the reverse search endpoint', NOW())
ON CONFLICT (key) DO NOTHING;
