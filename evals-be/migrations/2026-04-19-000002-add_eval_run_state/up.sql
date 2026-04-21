-- Per-run state machine rows for the stateless evals-be scoring pipeline.
--
-- Orch drives the loop: it POSTs to /api/evals/score/init, which persists an
-- eval_run_state row holding the current RunState JSONB blob and the
-- pending_step_id expected on the next /step call. Every /step advances the
-- state, rewrites the row, and either emits another CallLlm action or marks
-- the run Done.
--
-- The row is deleted on Done. Stale rows (LLM failures upstream, orch gave up
-- mid-loop) are cleaned up best-effort on a subsequent /init for the same
-- (summary_id, eval_version).

CREATE TABLE eval_run_state (
    run_id             UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    summary_id         UUID         NOT NULL,
    eval_version       TEXT         NOT NULL,
    -- Serialized RunState enum as JSON.
    state              JSONB        NOT NULL,
    -- Expected step_id + endpoint for the current outstanding LLM call.
    -- Both NULL when state is Done or no step is outstanding.
    pending_step_id    UUID,
    pending_endpoint   TEXT,
    created_at         TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ  NOT NULL DEFAULT now()
);

CREATE INDEX eval_run_state_summary_idx
    ON eval_run_state (summary_id, eval_version);

COMMENT ON TABLE  eval_run_state              IS 'In-flight stateful eval runs driven by an external loop (orch). Deleted on Done.';
COMMENT ON COLUMN eval_run_state.state        IS 'Serialized services::state_machine::RunState. JSONB so future variants can be added without schema changes.';
COMMENT ON COLUMN eval_run_state.pending_step_id IS 'step_id the next /step request must echo back. Used for idempotency / resync.';
