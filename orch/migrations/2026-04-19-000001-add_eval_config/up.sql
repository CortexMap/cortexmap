-- Phase-4 eval orchestrator configuration. Ships dark
-- (`eval_orchestrator_enabled = 'false'`) so the code can land before any
-- eval load hits the cluster. Flip the flag with a `PATCH /orch/api/config`.

INSERT INTO orch_config (key, value, description) VALUES
    ('eval_orchestrator_enabled',          'false', 'Master switch for the Phase-4 eval orchestrator background loop'),
    ('eval_orchestrator_poll_interval_secs', '60',  'Poll cadence for the eval orchestrator'),
    ('eval_orchestrator_concurrency',        '5',   'Max parallel POST /evals-be/api/evals/score calls'),
    ('evals_base_url',          'http://evals-be:8083', 'Base URL for evals-be'),
    ('eval_version',                       'v0.4.0',  'Cache version forwarded to evals-be on every score request')
ON CONFLICT (key) DO NOTHING;
