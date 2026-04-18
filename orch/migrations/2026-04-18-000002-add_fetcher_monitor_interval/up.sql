INSERT INTO orch_config (key, value, description) VALUES
    ('fetcher_monitor_interval_secs', '30', 'Fast-loop interval in seconds for re-probing worker health while the fetch queue is non-empty (default 30)'),
    ('enqueue_page_size', '20', 'Number of NCBI results per ESearch page in pipeline Phase 2 (default 20)')
ON CONFLICT (key) DO NOTHING;
