INSERT INTO orch_config (key, value, description) VALUES
    ('pipeline_cycle_sleep_secs', '3600', 'Sleep duration between pipeline cycles in seconds (default 1 hour)')
ON CONFLICT (key) DO NOTHING;
