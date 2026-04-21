//! Read-through `eval_scores` cache.
//!
//! Single entry-point for any code that wants to write a score row. The cache
//! is keyed by `(summary_hash, metric, eval_version)` — see migration
//! `2026-04-19-000001-create_eval_scores`.
//!
//! Centralising this here means a future metric impl cannot accidentally
//! bypass the cache: it simply doesn't have direct DB write access.

use crate::ServiceError;
use crate::infra::EvalsDatabase;
use domain::{EvalScore, NewEvalScore};
use std::error::Error;
use std::future::Future;
use uuid::Uuid;

/// Outcome of a `score_with_cache` call: the persisted row plus a flag telling
/// callers whether the score came from the cache (no compute) or was freshly
/// computed.
#[derive(Debug, Clone)]
pub struct CachedScore {
    pub row: EvalScore,
    pub cached: bool,
}

/// Read-through cache for a single `(summary_hash, metric, eval_version)`.
///
/// 1. SELECT on the unique cache index. On hit: return immediately, **no
///    `compute()` call**.
/// 2. On miss: invoke `compute()` to produce a `(score, judge_model, details)`
///    tuple, then INSERT ... ON CONFLICT DO NOTHING and re-select to resolve
///    concurrent writers to the same row.
pub async fn score_with_cache<DB, F, Fut, E>(
    db: &DB,
    database_url: &str,
    summary_id: Uuid,
    summary_hash: &str,
    metric: &str,
    eval_version: &str,
    compute: F,
) -> Result<CachedScore, ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ComputedScore, ServiceError<E>>>,
{
    if let Some(row) = db
        .lookup_score_by_hash(database_url, summary_hash, metric, eval_version)
        .await
        .map_err(ServiceError::InfraError)?
    {
        tracing::debug!(
            metric = metric,
            summary_hash = summary_hash,
            eval_version = eval_version,
            "metric=eval_cache_hit"
        );
        return Ok(CachedScore { row, cached: true });
    }

    let computed = compute().await?;

    let new = NewEvalScore {
        summary_id,
        summary_hash: summary_hash.to_string(),
        metric: metric.to_string(),
        score: computed.score,
        judge_model: computed.judge_model,
        details: computed.details,
        eval_version: eval_version.to_string(),
    };

    let row = db
        .insert_score(database_url, new)
        .await
        .map_err(ServiceError::InfraError)?;

    Ok(CachedScore { row, cached: false })
}

/// Result of the `compute` closure passed to `score_with_cache`.
#[derive(Debug, Clone)]
pub struct ComputedScore {
    pub score: f32,
    pub judge_model: Option<String>,
    pub details: Option<serde_json::Value>,
}

impl ComputedScore {
    pub fn structural(score: f32) -> Self {
        Self {
            score,
            judge_model: None,
            details: None,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the read-through cache. A minimal in-memory
    //! `EvalsDatabase` stub implements only the two methods `score_with_cache`
    //! actually calls (`lookup_score_by_hash`, `insert_score`). Every other
    //! trait method `unimplemented!()`s to make accidental reliance on them
    //! a loud test failure rather than silent mock-return drift.
    //!
    //! The stub also records the sequence of insert calls and the
    //! lookup-hit / lookup-miss counts so race-condition assertions can
    //! verify that the code re-selects after an `ON CONFLICT DO NOTHING`
    //! shortcut.
    use super::*;
    use crate::infra::{
        ChunkRow, EvalAggregate, EvalsDatabase, LoadedRunState, RetrievedChunk, SummaryRow,
        WorstOffenderRow,
    };
    use async_trait::async_trait;
    use chrono::NaiveDateTime;
    use domain::{EvalRun, EvalRunStatus};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, thiserror::Error)]
    #[error("mock infra error: {0}")]
    struct MockError(&'static str);

    /// Minimal stub: only `lookup_score_by_hash` and `insert_score` are
    /// implemented. All other methods panic on call — the cache layer must
    /// not touch anything else.
    #[derive(Default)]
    struct StubDb {
        scores: Mutex<Vec<EvalScore>>,
        insert_calls: AtomicUsize,
        lookup_calls: AtomicUsize,
        /// If `Some`, every call to `insert_score` returns this error.
        fail_insert: Mutex<Option<&'static str>>,
    }

    impl StubDb {
        fn seed(&self, row: EvalScore) {
            self.scores.lock().unwrap().push(row);
        }
    }

    fn make_row(summary_hash: &str, metric: &str, eval_version: &str, score: f32) -> EvalScore {
        EvalScore {
            id: Uuid::new_v4(),
            summary_id: Uuid::new_v4(),
            summary_hash: summary_hash.to_string(),
            metric: metric.to_string(),
            score,
            judge_model: None,
            details: None,
            eval_version: eval_version.to_string(),
            created_at: NaiveDateTime::default(),
        }
    }

    #[async_trait]
    impl EvalsDatabase for StubDb {
        type Error = MockError;

        async fn lookup_score_by_hash(
            &self,
            _database_url: &str,
            summary_hash: &str,
            metric: &str,
            eval_version: &str,
        ) -> Result<Option<EvalScore>, Self::Error> {
            self.lookup_calls.fetch_add(1, Ordering::SeqCst);
            let scores = self.scores.lock().unwrap();
            Ok(scores
                .iter()
                .find(|s| {
                    s.summary_hash == summary_hash
                        && s.metric == metric
                        && s.eval_version == eval_version
                })
                .cloned())
        }

        async fn insert_score(
            &self,
            _database_url: &str,
            new: NewEvalScore,
        ) -> Result<EvalScore, Self::Error> {
            self.insert_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(msg) = *self.fail_insert.lock().unwrap() {
                return Err(MockError(msg));
            }
            let mut scores = self.scores.lock().unwrap();
            // If a matching row already exists we return it unchanged —
            // modelling Postgres's `INSERT ... ON CONFLICT DO NOTHING`
            // followed by a `SELECT` to resolve the row. (The dedicated
            // race test uses its own RacingDb stub; StubDb only needs the
            // normal miss-insert path exercised here for the `miss` test.)
            if let Some(existing) = scores.iter().find(|s| {
                s.summary_hash == new.summary_hash
                    && s.metric == new.metric
                    && s.eval_version == new.eval_version
            }) {
                return Ok(existing.clone());
            }
            let row = EvalScore {
                id: Uuid::new_v4(),
                summary_id: new.summary_id,
                summary_hash: new.summary_hash,
                metric: new.metric,
                score: new.score,
                judge_model: new.judge_model,
                details: new.details,
                eval_version: new.eval_version,
                created_at: NaiveDateTime::default(),
            };
            scores.push(row.clone());
            Ok(row)
        }

        async fn get_summary(
            &self,
            _database_url: &str,
            _summary_id: Uuid,
        ) -> Result<Option<SummaryRow>, Self::Error> {
            unimplemented!("cache tests never hit get_summary")
        }

        async fn get_scores_for_summary(
            &self,
            _database_url: &str,
            _summary_id: Uuid,
        ) -> Result<Vec<EvalScore>, Self::Error> {
            unimplemented!("cache tests never hit get_scores_for_summary")
        }

        async fn get_eval_aggregate(
            &self,
            _database_url: &str,
            _eval_version: &str,
        ) -> Result<EvalAggregate, Self::Error> {
            unimplemented!("cache tests never hit get_eval_aggregate")
        }

        async fn get_worst_offenders(
            &self,
            _database_url: &str,
            _metric: &str,
            _eval_version: &str,
            _limit: i64,
        ) -> Result<Vec<WorstOffenderRow>, Self::Error> {
            unimplemented!("cache tests never hit get_worst_offenders")
        }

        async fn upsert_run(
            &self,
            _database_url: &str,
            _summary_id: Uuid,
            _eval_version: &str,
            _status: EvalRunStatus,
            _error_message: Option<String>,
        ) -> Result<EvalRun, Self::Error> {
            unimplemented!("cache tests never hit upsert_run")
        }

        async fn list_unscored_summary_ids(
            &self,
            _database_url: &str,
            _eval_version: &str,
            _limit: i64,
        ) -> Result<Vec<Uuid>, Self::Error> {
            unimplemented!("cache tests never hit list_unscored_summary_ids")
        }

        async fn retrieve_chunks_for_summary(
            &self,
            _database_url: &str,
            _summary_id: Uuid,
            _embedding: &[f32],
            _top_k: i64,
            _min_similarity: f32,
        ) -> Result<Vec<RetrievedChunk>, Self::Error> {
            unimplemented!("cache tests never hit retrieve_chunks_for_summary")
        }

        async fn load_chunks_by_ids(
            &self,
            _database_url: &str,
            _chunk_ids: &[Uuid],
        ) -> Result<Vec<ChunkRow>, Self::Error> {
            unimplemented!("cache tests never hit load_chunks_by_ids")
        }

        async fn insert_run_state(
            &self,
            _database_url: &str,
            _summary_id: Uuid,
            _eval_version: &str,
            _state: &serde_json::Value,
            _pending_step_id: Option<Uuid>,
            _pending_endpoint: Option<&str>,
        ) -> Result<Uuid, Self::Error> {
            unimplemented!("cache tests never hit insert_run_state")
        }

        async fn load_run_state(
            &self,
            _database_url: &str,
            _run_id: Uuid,
        ) -> Result<Option<LoadedRunState>, Self::Error> {
            unimplemented!("cache tests never hit load_run_state")
        }

        async fn save_run_state(
            &self,
            _database_url: &str,
            _run_id: Uuid,
            _state: &serde_json::Value,
            _pending_step_id: Option<Uuid>,
            _pending_endpoint: Option<&str>,
        ) -> Result<(), Self::Error> {
            unimplemented!("cache tests never hit save_run_state")
        }

        async fn delete_run_state(
            &self,
            _database_url: &str,
            _run_id: Uuid,
        ) -> Result<(), Self::Error> {
            unimplemented!("cache tests never hit delete_run_state")
        }

        async fn delete_run_states_for_summary(
            &self,
            _database_url: &str,
            _summary_id: Uuid,
            _eval_version: &str,
        ) -> Result<(), Self::Error> {
            unimplemented!("cache tests never hit delete_run_states_for_summary")
        }
    }

    const DB_URL: &str = "memory://";
    const METRIC: &str = "section_completeness";
    const VERSION: &str = "v-test";
    const HASH: &str = "deadbeef";

    /// Cache-hit path: a row already exists for the `(hash, metric, version)`
    /// triple, so `score_with_cache` must return `cached: true` and the
    /// scorer closure must NOT run (we detect this with a Mutex flag).
    #[tokio::test]
    async fn cache_hit_returns_without_invoking_scorer() {
        let db = StubDb::default();
        db.seed(make_row(HASH, METRIC, VERSION, 0.75));

        let scorer_ran = Mutex::new(false);
        let result = score_with_cache(
            &db,
            DB_URL,
            Uuid::new_v4(),
            HASH,
            METRIC,
            VERSION,
            || async {
                *scorer_ran.lock().unwrap() = true;
                Ok::<_, ServiceError<MockError>>(ComputedScore::structural(0.0))
            },
        )
        .await
        .expect("cache hit must succeed");

        assert!(result.cached, "expected cached=true on hit");
        assert!(
            (result.row.score - 0.75).abs() < 1e-6,
            "hit must return the seeded row's score"
        );
        assert!(
            !*scorer_ran.lock().unwrap(),
            "scorer closure must NOT run on a cache hit"
        );
        assert_eq!(
            db.insert_calls.load(Ordering::SeqCst),
            0,
            "no INSERT must be issued on a hit"
        );
        assert_eq!(
            db.lookup_calls.load(Ordering::SeqCst),
            1,
            "exactly one SELECT on a hit"
        );
    }

    /// Miss → compute → insert. Scorer closure must run, result must be
    /// `cached: false`, and exactly one INSERT must have been issued.
    #[tokio::test]
    async fn cache_miss_runs_scorer_and_inserts() {
        let db = StubDb::default();
        let summary_id = Uuid::new_v4();

        let result = score_with_cache(&db, DB_URL, summary_id, HASH, METRIC, VERSION, || async {
            Ok::<_, ServiceError<MockError>>(ComputedScore {
                score: 0.42,
                judge_model: Some("mock-judge".to_string()),
                details: Some(serde_json::json!({"note": "computed"})),
            })
        })
        .await
        .expect("miss path must succeed");

        assert!(!result.cached, "expected cached=false on miss");
        assert!((result.row.score - 0.42).abs() < 1e-6);
        assert_eq!(result.row.metric, METRIC);
        assert_eq!(result.row.summary_hash, HASH);
        assert_eq!(result.row.eval_version, VERSION);
        assert_eq!(result.row.summary_id, summary_id);
        assert_eq!(result.row.judge_model.as_deref(), Some("mock-judge"));
        assert_eq!(db.insert_calls.load(Ordering::SeqCst), 1);
        // Cache layer does NOT re-SELECT after insert on the no-conflict path —
        // the inserted row is returned directly.
        assert_eq!(db.lookup_calls.load(Ordering::SeqCst), 1);
    }

    /// Concurrent-writer race: initial lookup misses (a peer hasn't yet
    /// committed when we SELECT), scorer runs, then our INSERT ... ON
    /// CONFLICT DO NOTHING finds the peer's row already committed and the
    /// re-SELECT returns that winning row unchanged. We simulate this with
    /// a dedicated stub whose `lookup_score_by_hash` always returns `None`
    /// (the race window) and whose `insert_score` returns a pre-seeded row
    /// with a score distinct from what our scorer would produce — that way
    /// the assertion "we returned the winning row, not our own compute"
    /// is unambiguous.
    #[tokio::test]
    async fn concurrent_writer_insert_returns_existing_row() {
        // Custom stub: lookup always returns None (simulating miss), but
        // insert pretends a concurrent writer already landed a row — i.e.
        // returns an existing EvalScore with a deterministic id/score that
        // differs from what our scorer would produce.
        #[derive(Default)]
        struct RacingDb {
            existing: Mutex<Option<EvalScore>>,
            insert_calls: AtomicUsize,
        }

        #[async_trait]
        impl EvalsDatabase for RacingDb {
            type Error = MockError;

            async fn lookup_score_by_hash(
                &self,
                _database_url: &str,
                _summary_hash: &str,
                _metric: &str,
                _eval_version: &str,
            ) -> Result<Option<EvalScore>, Self::Error> {
                // First lookup: no row visible to this txn (race window).
                Ok(None)
            }

            async fn insert_score(
                &self,
                _database_url: &str,
                _new: NewEvalScore,
            ) -> Result<EvalScore, Self::Error> {
                self.insert_calls.fetch_add(1, Ordering::SeqCst);
                // The concrete impl executes INSERT ... ON CONFLICT DO NOTHING
                // RETURNING; when the conflict suppressed the insert, it
                // re-SELECTs and returns the row the winning writer left
                // behind. We emulate the final returned value here.
                Ok(self
                    .existing
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("test must seed the racing row"))
            }

            async fn get_summary(
                &self,
                _: &str,
                _: Uuid,
            ) -> Result<Option<SummaryRow>, Self::Error> {
                unimplemented!()
            }
            async fn get_scores_for_summary(
                &self,
                _: &str,
                _: Uuid,
            ) -> Result<Vec<EvalScore>, Self::Error> {
                unimplemented!()
            }
            async fn get_eval_aggregate(
                &self,
                _: &str,
                _: &str,
            ) -> Result<EvalAggregate, Self::Error> {
                unimplemented!()
            }
            async fn get_worst_offenders(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: i64,
            ) -> Result<Vec<WorstOffenderRow>, Self::Error> {
                unimplemented!()
            }
            async fn upsert_run(
                &self,
                _: &str,
                _: Uuid,
                _: &str,
                _: EvalRunStatus,
                _: Option<String>,
            ) -> Result<EvalRun, Self::Error> {
                unimplemented!()
            }
            async fn list_unscored_summary_ids(
                &self,
                _: &str,
                _: &str,
                _: i64,
            ) -> Result<Vec<Uuid>, Self::Error> {
                unimplemented!()
            }
            async fn retrieve_chunks_for_summary(
                &self,
                _: &str,
                _: Uuid,
                _: &[f32],
                _: i64,
                _: f32,
            ) -> Result<Vec<RetrievedChunk>, Self::Error> {
                unimplemented!()
            }
            async fn load_chunks_by_ids(
                &self,
                _: &str,
                _: &[Uuid],
            ) -> Result<Vec<ChunkRow>, Self::Error> {
                unimplemented!()
            }
            async fn insert_run_state(
                &self,
                _: &str,
                _: Uuid,
                _: &str,
                _: &serde_json::Value,
                _: Option<Uuid>,
                _: Option<&str>,
            ) -> Result<Uuid, Self::Error> {
                unimplemented!()
            }
            async fn load_run_state(
                &self,
                _: &str,
                _: Uuid,
            ) -> Result<Option<LoadedRunState>, Self::Error> {
                unimplemented!()
            }
            async fn save_run_state(
                &self,
                _: &str,
                _: Uuid,
                _: &serde_json::Value,
                _: Option<Uuid>,
                _: Option<&str>,
            ) -> Result<(), Self::Error> {
                unimplemented!()
            }
            async fn delete_run_state(&self, _: &str, _: Uuid) -> Result<(), Self::Error> {
                unimplemented!()
            }
            async fn delete_run_states_for_summary(
                &self,
                _: &str,
                _: Uuid,
                _: &str,
            ) -> Result<(), Self::Error> {
                unimplemented!()
            }
        }

        let db = RacingDb::default();
        let winning_row = make_row(HASH, METRIC, VERSION, 0.99);
        *db.existing.lock().unwrap() = Some(winning_row.clone());

        let result = score_with_cache(
            &db,
            DB_URL,
            winning_row.summary_id,
            HASH,
            METRIC,
            VERSION,
            || async {
                // Our scorer would compute 0.11, but the winning writer
                // already stored 0.99 — the race branch must return 0.99.
                Ok::<_, ServiceError<MockError>>(ComputedScore::structural(0.11))
            },
        )
        .await
        .expect("race path must resolve");

        // Current cache impl labels the race-resolved row as `cached: false`
        // because it took the insert branch. What matters is that the
        // returned row is the *winning* row, not the loser's computed value.
        assert!(
            (result.row.score - 0.99).abs() < 1e-6,
            "race must return the pre-existing winning row (0.99), got {}",
            result.row.score
        );
        assert_eq!(result.row.id, winning_row.id);
        assert_eq!(db.insert_calls.load(Ordering::SeqCst), 1);
    }

    /// If the scorer closure returns an error, `score_with_cache` must
    /// propagate it and NOT issue an insert.
    #[tokio::test]
    async fn scorer_error_propagates_without_insert() {
        let db = StubDb::default();

        let err = score_with_cache(
            &db,
            DB_URL,
            Uuid::new_v4(),
            HASH,
            METRIC,
            VERSION,
            || async {
                Err::<ComputedScore, _>(ServiceError::<MockError>::Other(
                    "scorer blew up".to_string(),
                ))
            },
        )
        .await
        .expect_err("scorer error must bubble up");

        match err {
            ServiceError::Other(msg) => assert_eq!(msg, "scorer blew up"),
            other => panic!("expected Other, got {:?}", other),
        }
        assert_eq!(
            db.insert_calls.load(Ordering::SeqCst),
            0,
            "insert must NOT run when scorer errors"
        );
        assert!(
            db.scores.lock().unwrap().is_empty(),
            "no row must be persisted on scorer error"
        );
    }

    /// If the infra-level INSERT fails, the infra error must be wrapped as
    /// `ServiceError::InfraError` and bubbled up (the persisted state is
    /// obviously unchanged in that case).
    #[tokio::test]
    async fn insert_error_wraps_infra_error() {
        let db = StubDb::default();
        *db.fail_insert.lock().unwrap() = Some("insert exploded");

        let err = score_with_cache(
            &db,
            DB_URL,
            Uuid::new_v4(),
            HASH,
            METRIC,
            VERSION,
            || async { Ok::<_, ServiceError<MockError>>(ComputedScore::structural(0.3)) },
        )
        .await
        .expect_err("insert failure must propagate");

        match err {
            ServiceError::InfraError(MockError(msg)) => assert_eq!(msg, "insert exploded"),
            other => panic!("expected InfraError, got {:?}", other),
        }
    }

    /// `ComputedScore::structural` must set `judge_model` and `details` to
    /// `None`. This is the shape contract for every deterministic structural
    /// metric (length, section completeness, etc.) and is relied on by the
    /// cache inserter — if these default to `Some(_)` the schema would reject.
    #[test]
    fn computed_score_structural_has_no_judge_model_or_details() {
        let c = ComputedScore::structural(0.42);
        assert!((c.score - 0.42).abs() < 1e-6);
        assert!(
            c.judge_model.is_none(),
            "structural metrics must carry no judge_model"
        );
        assert!(
            c.details.is_none(),
            "structural metrics must carry no details blob"
        );
    }

    /// Cache-hit path must surface the seeded row's judge_model and details
    /// verbatim — the wire layer relies on this to expose the original
    /// judge model for provenance in the UI. We seed a row carrying both
    /// fields and assert the hit returns them unchanged.
    #[tokio::test]
    async fn cache_hit_preserves_judge_model_and_details() {
        let db = StubDb::default();
        let mut row = make_row(HASH, METRIC, VERSION, 0.5);
        row.judge_model = Some("gpt-test-judge".to_string());
        row.details = Some(serde_json::json!({"rationale": "matches"}));
        db.seed(row.clone());

        let result = score_with_cache(
            &db,
            DB_URL,
            Uuid::new_v4(),
            HASH,
            METRIC,
            VERSION,
            || async { Ok::<_, ServiceError<MockError>>(ComputedScore::structural(0.0)) },
        )
        .await
        .expect("hit must succeed");

        assert!(result.cached);
        assert_eq!(result.row.judge_model.as_deref(), Some("gpt-test-judge"));
        assert_eq!(
            result.row.details,
            Some(serde_json::json!({"rationale": "matches"}))
        );
        assert_eq!(
            db.insert_calls.load(Ordering::SeqCst),
            0,
            "no INSERT on hit even when judge_model/details are populated"
        );
    }

    /// Two different `eval_version`s for the same `(summary_hash, metric)`
    /// must be stored as separate rows: an upgrade of the eval pipeline
    /// bumps the version and re-scores every summary. We seed `v1` and ask
    /// for `v2` — the lookup must miss, the scorer must run, and a second
    /// row must land alongside the first.
    #[tokio::test]
    async fn cache_miss_when_eval_version_differs() {
        let db = StubDb::default();
        // Seed a v1 row.
        db.seed(make_row(HASH, METRIC, "v1", 0.3));

        let result = score_with_cache(&db, DB_URL, Uuid::new_v4(), HASH, METRIC, "v2", || async {
            Ok::<_, ServiceError<MockError>>(ComputedScore::structural(0.7))
        })
        .await
        .expect("v2 must compute fresh");

        assert!(!result.cached, "v2 must miss and compute");
        assert!((result.row.score - 0.7).abs() < 1e-6);
        assert_eq!(result.row.eval_version, "v2");
        assert_eq!(db.insert_calls.load(Ordering::SeqCst), 1);
        // Both rows coexist.
        let scores = db.scores.lock().unwrap();
        assert_eq!(scores.len(), 2);
        let versions: std::collections::HashSet<_> =
            scores.iter().map(|s| s.eval_version.clone()).collect();
        assert!(versions.contains("v1"));
        assert!(versions.contains("v2"));
    }

    /// Two different `metric`s for the same `(summary_hash, eval_version)`
    /// must also miss independently — the unique index is on the triple.
    #[tokio::test]
    async fn cache_miss_when_metric_differs() {
        let db = StubDb::default();
        db.seed(make_row(HASH, "section_completeness", VERSION, 0.6));

        let result = score_with_cache(
            &db,
            DB_URL,
            Uuid::new_v4(),
            HASH,
            "length_in_range",
            VERSION,
            || async { Ok::<_, ServiceError<MockError>>(ComputedScore::structural(0.9)) },
        )
        .await
        .expect("different metric must miss");

        assert!(!result.cached);
        assert_eq!(result.row.metric, "length_in_range");
        assert!((result.row.score - 0.9).abs() < 1e-6);
        assert_eq!(db.insert_calls.load(Ordering::SeqCst), 1);
        assert_eq!(db.scores.lock().unwrap().len(), 2);
    }

    /// Miss path: scorer produces details JSON — the cache layer must pass
    /// it through to `NewEvalScore.details` so the persisted row carries
    /// the computed metadata (rationales, sub-scores, etc.). Guards against
    /// a regression where the details field is silently dropped.
    #[tokio::test]
    async fn cache_miss_persists_scorer_details_and_judge_model() {
        let db = StubDb::default();
        let details = serde_json::json!({"breakdown": [1, 2, 3], "note": "ok"});

        let result = score_with_cache(&db, DB_URL, Uuid::new_v4(), HASH, METRIC, VERSION, || {
            let d = details.clone();
            async move {
                Ok::<_, ServiceError<MockError>>(ComputedScore {
                    score: 0.55,
                    judge_model: Some("custom-judge-v3".to_string()),
                    details: Some(d),
                })
            }
        })
        .await
        .expect("miss must persist details");

        assert!(!result.cached);
        assert_eq!(result.row.judge_model.as_deref(), Some("custom-judge-v3"));
        assert_eq!(result.row.details, Some(details));
    }
}
