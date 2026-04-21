-- Tables that hold per-summary evaluation results.
--
-- `eval_scores` is keyed by (summary_hash, metric, eval_version), which acts
-- as the cache: a re-evaluation of identical summary text returns the existing
-- row instead of re-running the metric. Bump `eval_version` to force a refresh
-- across the corpus when scoring logic changes.
--
-- `eval_runs` tracks the per-summary lifecycle so the orchestrator can answer
-- "is this summary evaluated?" without having to count score rows.

CREATE TABLE eval_scores (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    summary_id      UUID         NOT NULL REFERENCES region_summary(id) ON DELETE CASCADE,
    summary_hash    VARCHAR(64)  NOT NULL,
    metric          VARCHAR(64)  NOT NULL,
    score           REAL         NOT NULL,
    judge_model     VARCHAR(128),
    details         JSONB,
    eval_version    VARCHAR(16)  NOT NULL,
    created_at      TIMESTAMP    NOT NULL DEFAULT NOW()
);

-- Cache key: identical summary text yields identical scores per (metric, eval_version).
CREATE UNIQUE INDEX ix_eval_scores_cache
    ON eval_scores (summary_hash, metric, eval_version);

CREATE INDEX ix_eval_scores_summary
    ON eval_scores (summary_id);

CREATE INDEX ix_eval_scores_metric_score
    ON eval_scores (metric, score);

COMMENT ON TABLE  eval_scores              IS 'Per-(summary_hash, metric, eval_version) eval result. Indexed unique key acts as a content-addressed cache; identical summary text never recomputes a score.';
COMMENT ON COLUMN eval_scores.summary_hash IS 'SHA-256 hex digest of region_summary.summary at score time. Same hash + same metric + same eval_version = cache hit.';
COMMENT ON COLUMN eval_scores.score        IS 'Normalized score in [0.0, 1.0].';
COMMENT ON COLUMN eval_scores.details      IS 'Optional JSON payload (e.g. per-claim verdicts for groundedness, per-criterion rationales for rubric).';
COMMENT ON COLUMN eval_scores.eval_version IS 'Bump to force re-evaluation of every summary across the corpus.';


CREATE TABLE eval_runs (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    summary_id      UUID         NOT NULL REFERENCES region_summary(id) ON DELETE CASCADE,
    eval_version    VARCHAR(16)  NOT NULL,
    status          VARCHAR(16)  NOT NULL,  -- 'queued' | 'running' | 'complete' | 'failed'
    error_message   TEXT,
    started_at      TIMESTAMP,
    completed_at    TIMESTAMP,
    created_at      TIMESTAMP    NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX ix_eval_runs_unique
    ON eval_runs (summary_id, eval_version);

CREATE INDEX ix_eval_runs_status
    ON eval_runs (status);

COMMENT ON TABLE  eval_runs        IS 'Lifecycle marker per (summary_id, eval_version). Lets orch answer "is this summary scored?" without scanning eval_scores.';
COMMENT ON COLUMN eval_runs.status IS 'queued | running | complete | failed';
