DELETE FROM orch_config WHERE key IN (
    'eval_orchestrator_enabled',
    'eval_orchestrator_poll_interval_secs',
    'eval_orchestrator_concurrency',
    'evals_base_url',
    'eval_version'
);
