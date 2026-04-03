-- Add ingestion scheduler configuration
INSERT INTO orch_config (key, value, description) VALUES
    ('ingestion_interval_secs', '3600', 'Interval in seconds between periodic ingestion cycles (default: 1 hour)'),
    ('ingestion_batch_size', '0', 'Maximum number of regions to process per ingestion cycle (0 = all regions)'),
    ('ingestion_enabled', 'true', 'Whether periodic knowledge base ingestion is enabled')
ON CONFLICT (key) DO NOTHING;
