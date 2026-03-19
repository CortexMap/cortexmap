-- Add fetcher retry configuration to orch_config
INSERT INTO orch_config (key, value, description) VALUES
    ('fetcher_task_timeout_secs', '2', 'Default task timeout in seconds for fetcher workers'),
    ('fetcher_max_retry_attempts', '3', 'Default max retry attempts for fetcher task components'),
    ('fetcher_backoff_strategy', 'constant', 'Backoff strategy: constant, linear, exponential, fibonacci'),
    ('fetcher_max_delay_secs', '60', 'Maximum backoff delay in seconds (for linear/exponential/fibonacci)'),
    ('fetcher_backoff_jitter', '0.0', 'Jitter factor 0.0-1.0 for exponential backoff randomization'),
    ('fetcher_empty_queue_sleep_secs', '5', 'Sleep duration in seconds when fetcher queue is empty'),
    ('fetcher_stale_task_multiplier', '10', 'Multiplier for stale task detection (task_timeout * multiplier)'),
    ('fetcher_summary_max_retries', '', 'Max retries for summary component (empty = use global)'),
    ('fetcher_abstract_max_retries', '', 'Max retries for abstract component (empty = use global)'),
    ('fetcher_pdf_max_retries', '', 'Max retries for PDF component (empty = use global)')
ON CONFLICT (key) DO NOTHING;
