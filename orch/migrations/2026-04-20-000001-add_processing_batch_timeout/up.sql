-- Seeds the processing_batch_timeout_secs config key used by the
-- stale-batch watcher to recover batches stuck in 'processing' state
-- (typically after a brainatlas-be restart drops in-flight RAG work).
INSERT INTO orch_config (key, value, description)
VALUES (
    'processing_batch_timeout_secs',
    '1800',
    'Max seconds a batch may stay in ''processing'' before the stale-batch watcher marks it failed. Default 1800 = 30 minutes.'
)
ON CONFLICT (key) DO NOTHING;
