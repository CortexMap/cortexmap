DROP INDEX IF EXISTS ix_eval_runs_status;
DROP INDEX IF EXISTS ix_eval_runs_unique;
DROP TABLE IF EXISTS eval_runs;

DROP INDEX IF EXISTS ix_eval_scores_metric_score;
DROP INDEX IF EXISTS ix_eval_scores_summary;
DROP INDEX IF EXISTS ix_eval_scores_cache;
DROP TABLE IF EXISTS eval_scores;
