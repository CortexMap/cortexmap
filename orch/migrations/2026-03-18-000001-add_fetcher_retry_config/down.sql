-- Remove fetcher retry configuration from orch_config
DELETE FROM orch_config WHERE key IN (
    'fetcher_task_timeout_secs',
    'fetcher_max_retry_attempts',
    'fetcher_backoff_strategy',
    'fetcher_max_delay_secs',
    'fetcher_backoff_jitter',
    'fetcher_empty_queue_sleep_secs',
    'fetcher_stale_task_multiplier',
    'fetcher_summary_max_retries',
    'fetcher_abstract_max_retries',
    'fetcher_pdf_max_retries'
);
