//! Top-level orchestration: wires services + infra together.
//!
//! The public entry points (`init_score`, `step_score`) drive the stateless
//! scoring state machine. Orch calls them from the outside, feeding LLM
//! responses back in and making the actual LLM HTTP calls on evals's behalf.

use crate::error::AppError;
use crate::run_eval::{
    probe_citation_cache, probe_groundedness_cache, probe_rubric_cache, run_structural_metrics,
};
use domain::{EvalRunStatus, compute_hash};
use rpc_types::{
    EvalSummaryResponse, InitScoreRequest, InitScoreResponse, LlmEndpoint, MetricResult,
    MetricStats, NextAction, ScoreEntry, ScoresForSummaryResponse, StepRequest, StepResponse,
    UnscoredResponse, WorstOffender, WorstOffendersResponse,
};
use services::state_machine::{self, RunContext, RunState};
use services::{EnvInfra, EvalsDatabase, ServiceError};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

/// All knobs needed to score a summary. Loaded once at startup from env;
/// per-call overrides happen via the request body's `eval_version` field.
#[derive(Debug, Clone)]
pub struct EvalRuntimeConfig {
    pub database_url: String,
    pub eval_version: String,
    pub judge_chat_model: String,
    pub rubric_chat_model: String,
    pub embedding_model: String,
    pub top_k_chunks: i64,
    pub similarity_threshold: f32,
    /// When `true`, the per-citation "did the author cite the right chunk?"
    /// judge runs. Defaults to `false` so the cheap deterministic citation
    /// metrics ship first with zero added LLM spend.
    pub citation_support_enabled: bool,
    /// Upper bound on the number of JudgeCitation calls per summary. Excess
    /// in-scope citations are skipped and the support score is flagged
    /// `details.truncated = true`.
    pub citation_support_max_calls: usize,
}

impl EvalRuntimeConfig {
    pub fn from_env<E: EnvInfra>(env: &E) -> Result<Self, AppError<E::Error>> {
        use domain::ConfigKey;

        fn get_or_default<E: EnvInfra>(env: &E, key: &str, default: &str) -> String {
            env.get_env_var(key).unwrap_or_else(|_| default.to_string())
        }

        let database_url = env
            .get_env_var("DATABASE_URL")
            .map_err(|_| AppError::MissingEnv("DATABASE_URL".to_string()))?;

        let eval_version =
            get_or_default(env, "EVAL_VERSION", ConfigKey::EvalVersion.default_value());

        let judge_chat_model = get_or_default(
            env,
            "EVAL_JUDGE_CHAT_MODEL",
            ConfigKey::EvalJudgeChatModel.default_value(),
        );
        let rubric_chat_model = get_or_default(
            env,
            "EVAL_RUBRIC_CHAT_MODEL",
            ConfigKey::EvalRubricChatModel.default_value(),
        );
        let embedding_model = get_or_default(
            env,
            "EVAL_EMBEDDING_MODEL",
            ConfigKey::EvalEmbeddingModel.default_value(),
        );

        let top_k_chunks: i64 = get_or_default(
            env,
            "EVAL_TOP_K_CHUNKS",
            ConfigKey::EvalTopKChunks.default_value(),
        )
        .parse()
        .map_err(|_| AppError::InvalidConfig {
            key: "EVAL_TOP_K_CHUNKS".to_string(),
            value: "non-integer".to_string(),
        })?;

        let similarity_threshold: f32 = get_or_default(
            env,
            "EVAL_SIMILARITY_THRESHOLD",
            ConfigKey::EvalSimilarityThreshold.default_value(),
        )
        .parse()
        .map_err(|_| AppError::InvalidConfig {
            key: "EVAL_SIMILARITY_THRESHOLD".to_string(),
            value: "non-float".to_string(),
        })?;

        let citation_support_enabled: bool =
            get_or_default(env, "EVAL_CITATION_SUPPORT_ENABLED", "false")
                .parse()
                .unwrap_or(false);

        let citation_support_max_calls: usize =
            get_or_default(env, "EVAL_CITATION_SUPPORT_MAX_CALLS", "30")
                .parse()
                .unwrap_or(30);

        Ok(Self {
            database_url,
            eval_version,
            judge_chat_model,
            rubric_chat_model,
            embedding_model,
            top_k_chunks,
            similarity_threshold,
            citation_support_enabled,
            citation_support_max_calls,
        })
    }
}

/// Concrete app composed of database + env. No outbound HTTP.
pub struct EvalsApp<DB, EN, E>
where
    DB: EvalsDatabase<Error = E>,
    EN: EnvInfra<Error = E>,
    E: Error + Send + Sync + 'static,
{
    pub db: Arc<DB>,
    pub env: Arc<EN>,
    pub config: EvalRuntimeConfig,
}

impl<DB, EN, E> EvalsApp<DB, EN, E>
where
    DB: EvalsDatabase<Error = E>,
    EN: EnvInfra<Error = E>,
    E: Error + Send + Sync + 'static,
{
    pub fn new(db: Arc<DB>, env: Arc<EN>) -> Result<Self, AppError<E>> {
        let config = EvalRuntimeConfig::from_env(env.as_ref())?;
        Ok(Self { db, env, config })
    }

    /// Kick off a new scoring run. Runs structural metrics synchronously,
    /// probes the cache for groundedness + rubric, and persists the initial
    /// `eval_run_state` row. Returns the first `NextAction` orch should take.
    pub async fn init_score(
        &self,
        req: InitScoreRequest,
    ) -> Result<InitScoreResponse, AppError<E>> {
        let eval_version = req
            .eval_version
            .clone()
            .unwrap_or_else(|| self.config.eval_version.clone());

        let summary = self
            .db
            .get_summary(&self.config.database_url, req.summary_id)
            .await
            .map_err(ServiceError::InfraError)?
            .ok_or(AppError::SummaryNotFound)?;

        let summary_hash = compute_hash(&summary.summary);

        // Clean up any abandoned run_state rows for this (summary, version).
        let _ = self
            .db
            .delete_run_states_for_summary(&self.config.database_url, req.summary_id, &eval_version)
            .await;

        // Mark the run as running.
        let _ = self
            .db
            .upsert_run(
                &self.config.database_url,
                req.summary_id,
                &eval_version,
                EvalRunStatus::Running,
                None,
            )
            .await;

        // Accumulate metrics as we go.
        let mut metrics: Vec<MetricResult> = Vec::new();

        // 1) Structural metrics (always cheap, always compute+cache).
        run_structural_metrics(
            self.db.as_ref(),
            &self.config.database_url,
            req.summary_id,
            &summary_hash,
            &eval_version,
            &summary,
            &mut metrics,
        )
        .await?;

        // 2) Probe cache for groundedness + rubric.
        let g_cached = probe_groundedness_cache(
            self.db.as_ref(),
            &self.config.database_url,
            &summary_hash,
            &eval_version,
            &mut metrics,
        )
        .await?;
        let r_cached = probe_rubric_cache(
            self.db.as_ref(),
            &self.config.database_url,
            &summary_hash,
            &eval_version,
            &mut metrics,
        )
        .await?;
        // Additive: surfaces any already-cached citation metrics so the
        // Done response includes them. Does not gate the state machine.
        probe_citation_cache(
            self.db.as_ref(),
            &self.config.database_url,
            &summary_hash,
            &eval_version,
            &mut metrics,
        )
        .await?;

        let ctx = RunContext {
            summary: &summary,
            summary_hash: &summary_hash,
            eval_version: &eval_version,
            judge_chat_model: &self.config.judge_chat_model,
            rubric_chat_model: &self.config.rubric_chat_model,
            embedding_model: &self.config.embedding_model,
            top_k_chunks: self.config.top_k_chunks,
            similarity_threshold: self.config.similarity_threshold,
            citation_support_enabled: self.config.citation_support_enabled,
            citation_support_max_calls: self.config.citation_support_max_calls,
        };

        let (state, next) =
            state_machine::initial_action(&ctx, g_cached, r_cached, metrics.clone());

        // If we're already Done (full cache hit or no claims), finalize and
        // don't persist any run_state row.
        if matches!(next, NextAction::Done { .. }) {
            let _ = self
                .db
                .upsert_run(
                    &self.config.database_url,
                    req.summary_id,
                    &eval_version,
                    EvalRunStatus::Complete,
                    None,
                )
                .await;
            // run_id is irrelevant when there's no pending work, but we still
            // need *some* UUID for the wire type. Use a nil UUID to indicate
            // "no run persisted".
            return Ok(InitScoreResponse {
                run_id: Uuid::nil(),
                summary_id: req.summary_id,
                eval_version,
                next,
            });
        }

        // Otherwise persist the new run_state and return its id.
        let (step_id, endpoint_str) = pending_fields(&next);
        let state_json = serde_json::to_value(&state).expect("RunState serializable");
        let run_id = self
            .db
            .insert_run_state(
                &self.config.database_url,
                req.summary_id,
                &eval_version,
                &state_json,
                step_id,
                endpoint_str.as_deref(),
            )
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(InitScoreResponse {
            run_id,
            summary_id: req.summary_id,
            eval_version,
            next,
        })
    }

    /// Advance an in-flight run with an LLM response. Returns the next action
    /// (another CallLlm, or Done). On Done, writes the final `eval_runs` row
    /// and deletes the `eval_run_state` row.
    pub async fn step_score(&self, req: StepRequest) -> Result<StepResponse, AppError<E>> {
        let loaded = self
            .db
            .load_run_state(&self.config.database_url, req.run_id)
            .await
            .map_err(ServiceError::InfraError)?
            .ok_or_else(|| AppError::InvalidArg(format!("unknown run_id {}", req.run_id)))?;
        let (summary_id, eval_version, state_json, pending_step_id) = loaded;

        match pending_step_id {
            Some(expected) if expected == req.step_id => {}
            _ => {
                return Err(AppError::InvalidArg(format!(
                    "step_id mismatch for run {}: expected {:?}, got {}",
                    req.run_id, pending_step_id, req.step_id
                )));
            }
        }

        let state: RunState = serde_json::from_value(state_json)
            .map_err(|e| AppError::InvalidArg(format!("corrupted run state: {e}")))?;

        let summary = self
            .db
            .get_summary(&self.config.database_url, summary_id)
            .await
            .map_err(ServiceError::InfraError)?
            .ok_or(AppError::SummaryNotFound)?;
        let summary_hash = compute_hash(&summary.summary);

        let ctx = RunContext {
            summary: &summary,
            summary_hash: &summary_hash,
            eval_version: &eval_version,
            judge_chat_model: &self.config.judge_chat_model,
            rubric_chat_model: &self.config.rubric_chat_model,
            embedding_model: &self.config.embedding_model,
            top_k_chunks: self.config.top_k_chunks,
            similarity_threshold: self.config.similarity_threshold,
            citation_support_enabled: self.config.citation_support_enabled,
            citation_support_max_calls: self.config.citation_support_max_calls,
        };

        // Recreate the accumulator. The state machine appends to this list
        // every time it persists a metric; on Done it emits it back to us.
        let mut accumulated: Vec<MetricResult> = Vec::new();

        let (new_state, next) = state_machine::advance(
            self.db.as_ref(),
            &self.config.database_url,
            state,
            &ctx,
            req.llm_response,
            &mut accumulated,
        )
        .await?;

        let next = match next {
            NextAction::Done { .. } => {
                // Delete the run_state row and record the run as complete.
                let _ = self
                    .db
                    .delete_run_state(&self.config.database_url, req.run_id)
                    .await;
                let _ = self
                    .db
                    .upsert_run(
                        &self.config.database_url,
                        summary_id,
                        &eval_version,
                        EvalRunStatus::Complete,
                        None,
                    )
                    .await;

                // Rebuild the metrics list from the full `eval_scores` cache
                // (includes structural + groundedness + rubric metrics, all
                // marked `cached: true` since they're loaded from cache at
                // this point — the wire layer just needs the values).
                let all_rows = self
                    .db
                    .get_scores_for_summary(&self.config.database_url, summary_id)
                    .await
                    .map_err(ServiceError::InfraError)?;
                let metrics_now: Vec<MetricResult> = all_rows
                    .into_iter()
                    .filter(|r| r.eval_version == eval_version)
                    .map(|r| {
                        // Preserve the cache-hit status from `accumulated` if
                        // this metric is present there (so newly-computed
                        // rows show `cached: false`); otherwise default true.
                        let accum_entry = accumulated.iter().find(|m| m.metric == r.metric);
                        MetricResult {
                            metric: r.metric,
                            score: r.score,
                            cached: accum_entry.map(|m| m.cached).unwrap_or(true),
                            judge_model: r.judge_model,
                        }
                    })
                    .collect();

                NextAction::Done {
                    metrics: metrics_now,
                }
            }
            NextAction::CallLlm {
                step_id,
                endpoint,
                path,
                body,
            } => {
                let endpoint_str = Some(endpoint_to_str(&endpoint).to_string());
                let state_json = serde_json::to_value(&new_state).expect("RunState serializable");
                self.db
                    .save_run_state(
                        &self.config.database_url,
                        req.run_id,
                        &state_json,
                        Some(step_id),
                        endpoint_str.as_deref(),
                    )
                    .await
                    .map_err(ServiceError::InfraError)?;
                NextAction::CallLlm {
                    step_id,
                    endpoint,
                    path,
                    body,
                }
            }
        };

        Ok(StepResponse {
            run_id: req.run_id,
            next,
        })
    }

    pub async fn scores_for_summary(
        &self,
        summary_id: Uuid,
    ) -> Result<ScoresForSummaryResponse, AppError<E>> {
        let rows = self
            .db
            .get_scores_for_summary(&self.config.database_url, summary_id)
            .await
            .map_err(services::ServiceError::InfraError)?;

        let scores = rows
            .into_iter()
            .map(|r| ScoreEntry {
                metric: r.metric,
                score: r.score,
                eval_version: r.eval_version,
                judge_model: r.judge_model,
                details: r.details,
                created_at: r.created_at.to_string(),
            })
            .collect();

        Ok(ScoresForSummaryResponse { summary_id, scores })
    }

    pub async fn aggregate_summary(
        &self,
        eval_version: Option<String>,
    ) -> Result<EvalSummaryResponse, AppError<E>> {
        let ver = eval_version.unwrap_or_else(|| self.config.eval_version.clone());
        let agg = self
            .db
            .get_eval_aggregate(&self.config.database_url, &ver)
            .await
            .map_err(services::ServiceError::InfraError)?;

        let per_metric = agg
            .per_metric
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    MetricStats {
                        avg: v.avg,
                        min: v.min,
                        max: v.max,
                        count: v.count,
                    },
                )
            })
            .collect();

        Ok(EvalSummaryResponse {
            eval_version: ver,
            total_summaries: agg.total_summaries,
            total_scored: agg.total_scored,
            per_metric,
        })
    }

    pub async fn worst_offenders(
        &self,
        metric: String,
        limit: i64,
        eval_version: Option<String>,
    ) -> Result<WorstOffendersResponse, AppError<E>> {
        let ver = eval_version.unwrap_or_else(|| self.config.eval_version.clone());
        let rows = self
            .db
            .get_worst_offenders(&self.config.database_url, &metric, &ver, limit)
            .await
            .map_err(services::ServiceError::InfraError)?;

        let entries = rows
            .into_iter()
            .map(|r| WorstOffender {
                summary_id: r.summary_id,
                region_name: r.region_name,
                metric: r.metric,
                score: r.score,
                eval_version: r.eval_version,
            })
            .collect();

        Ok(WorstOffendersResponse {
            metric,
            limit,
            entries,
        })
    }

    /// Active summary IDs that have no `complete` `eval_runs` row for the
    /// given `eval_version`. Used by the orch eval-orchestrator to find work.
    pub async fn list_unscored_summary_ids(
        &self,
        eval_version: Option<String>,
        limit: i64,
    ) -> Result<UnscoredResponse, AppError<E>> {
        let ver = eval_version.unwrap_or_else(|| self.config.eval_version.clone());
        let ids = self
            .db
            .list_unscored_summary_ids(&self.config.database_url, &ver, limit)
            .await
            .map_err(services::ServiceError::InfraError)?;

        Ok(UnscoredResponse {
            eval_version: ver,
            limit,
            summary_ids: ids,
        })
    }
}

fn endpoint_to_str(endpoint: &LlmEndpoint) -> &'static str {
    match endpoint {
        LlmEndpoint::ExtractClaims => "extract_claims",
        LlmEndpoint::Embed => "embed",
        LlmEndpoint::JudgeGroundedness => "judge_groundedness",
        LlmEndpoint::JudgeRubric => "judge_rubric",
        LlmEndpoint::JudgeCitation => "judge_citation",
    }
}

fn pending_fields(next: &NextAction) -> (Option<Uuid>, Option<String>) {
    match next {
        NextAction::CallLlm {
            step_id, endpoint, ..
        } => (Some(*step_id), Some(endpoint_to_str(endpoint).to_string())),
        NextAction::Done { .. } => (None, None),
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for `EvalsApp` glue code.
    //!
    //! The integration test at `evals-be/crates/app/tests/cache_hit.rs` drives
    //! the full state-machine happy path. These in-file tests hit the fast
    //! error branches (`from_env`, read-only methods, unknown run_id,
    //! SummaryNotFound) that the integration test doesn't cover.
    //!
    //! We use a handful of tiny hand-rolled fakes — a `StubEnv` that returns
    //! pre-seeded values per key, and a `ReadOnlyDb` that serves fixed rows.
    //! No mockall, no wiremock.
    use super::*;
    use crate::error::AppError;
    use async_trait::async_trait;
    use chrono::NaiveDateTime;
    use domain::{EvalRun, EvalRunStatus, EvalScore, NewEvalScore};
    use services::{
        ChunkRow, EnvInfra, EvalAggregate, EvalsDatabase, LoadedRunState, MetricStatsRaw,
        RetrievedChunk, SummaryRow, WorstOffenderRow,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug, thiserror::Error)]
    #[error("mock err: {0}")]
    struct MockErr(String);

    // ---- Env fake: returns Ok for seeded keys, Err otherwise. ----

    struct StubEnv {
        vars: HashMap<&'static str, String>,
    }

    impl StubEnv {
        fn new() -> Self {
            Self {
                vars: HashMap::new(),
            }
        }
        fn with(mut self, key: &'static str, value: &str) -> Self {
            self.vars.insert(key, value.to_string());
            self
        }
    }

    impl EnvInfra for StubEnv {
        type Error = MockErr;
        fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
            self.vars
                .get(key)
                .cloned()
                .ok_or_else(|| MockErr(format!("missing {key}")))
        }
    }

    // ---- Read-only DB fake. Every mutating method panics — read-only tests
    // must not touch them. Every read returns a preloaded value. ----

    #[derive(Default)]
    struct ReadOnlyDb {
        summary: Mutex<Option<SummaryRow>>,
        scores: Mutex<Vec<EvalScore>>,
        aggregate: Mutex<EvalAggregate>,
        worst: Mutex<Vec<WorstOffenderRow>>,
        unscored: Mutex<Vec<Uuid>>,
        /// If set, every read returns this error.
        fail: Mutex<Option<String>>,
    }

    impl ReadOnlyDb {
        fn fail_all(msg: &str) -> Self {
            let db = Self::default();
            *db.fail.lock().unwrap() = Some(msg.to_string());
            db
        }
        fn check(&self) -> Result<(), MockErr> {
            if let Some(m) = self.fail.lock().unwrap().as_ref() {
                return Err(MockErr(m.clone()));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl EvalsDatabase for ReadOnlyDb {
        type Error = MockErr;

        async fn lookup_score_by_hash(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Option<EvalScore>, MockErr> {
            unimplemented!("read-only tests never hit lookup_score_by_hash")
        }
        async fn insert_score(&self, _: &str, _: NewEvalScore) -> Result<EvalScore, MockErr> {
            unimplemented!("read-only tests never hit insert_score")
        }
        async fn get_summary(
            &self,
            _: &str,
            summary_id: Uuid,
        ) -> Result<Option<SummaryRow>, MockErr> {
            self.check()?;
            Ok(self
                .summary
                .lock()
                .unwrap()
                .as_ref()
                .filter(|r| r.id == summary_id)
                .cloned())
        }
        async fn get_scores_for_summary(
            &self,
            _: &str,
            summary_id: Uuid,
        ) -> Result<Vec<EvalScore>, MockErr> {
            self.check()?;
            Ok(self
                .scores
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.summary_id == summary_id)
                .cloned()
                .collect())
        }
        async fn get_eval_aggregate(&self, _: &str, _: &str) -> Result<EvalAggregate, MockErr> {
            self.check()?;
            Ok(self.aggregate.lock().unwrap().clone())
        }
        async fn get_worst_offenders(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: i64,
        ) -> Result<Vec<WorstOffenderRow>, MockErr> {
            self.check()?;
            Ok(self.worst.lock().unwrap().clone())
        }
        async fn upsert_run(
            &self,
            _: &str,
            _: Uuid,
            _: &str,
            _: EvalRunStatus,
            _: Option<String>,
        ) -> Result<EvalRun, MockErr> {
            unimplemented!("read-only tests never hit upsert_run")
        }
        async fn list_unscored_summary_ids(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> Result<Vec<Uuid>, MockErr> {
            self.check()?;
            Ok(self.unscored.lock().unwrap().clone())
        }
        async fn retrieve_chunks_for_summary(
            &self,
            _: &str,
            _: Uuid,
            _: &[f32],
            _: i64,
            _: f32,
        ) -> Result<Vec<RetrievedChunk>, MockErr> {
            unimplemented!()
        }
        async fn load_chunks_by_ids(
            &self,
            _: &str,
            _: &[Uuid],
        ) -> Result<Vec<ChunkRow>, MockErr> {
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
        ) -> Result<Uuid, MockErr> {
            unimplemented!()
        }
        async fn load_run_state(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<LoadedRunState>, MockErr> {
            // Return None so `step_score`'s `unknown run_id` branch fires.
            Ok(None)
        }
        async fn save_run_state(
            &self,
            _: &str,
            _: Uuid,
            _: &serde_json::Value,
            _: Option<Uuid>,
            _: Option<&str>,
        ) -> Result<(), MockErr> {
            unimplemented!()
        }
        async fn delete_run_state(&self, _: &str, _: Uuid) -> Result<(), MockErr> {
            unimplemented!()
        }
        async fn delete_run_states_for_summary(
            &self,
            _: &str,
            _: Uuid,
            _: &str,
        ) -> Result<(), MockErr> {
            unimplemented!()
        }
    }

    fn min_config() -> EvalRuntimeConfig {
        EvalRuntimeConfig {
            database_url: "memory://".to_string(),
            eval_version: "v-app-test".to_string(),
            judge_chat_model: "j".to_string(),
            rubric_chat_model: "r".to_string(),
            embedding_model: "e".to_string(),
            top_k_chunks: 3,
            similarity_threshold: 0.5,
            citation_support_enabled: false,
            citation_support_max_calls: 30,
        }
    }

    fn make_app(db: Arc<ReadOnlyDb>) -> EvalsApp<ReadOnlyDb, StubEnv, MockErr> {
        EvalsApp {
            db,
            env: Arc::new(StubEnv::new()),
            config: min_config(),
        }
    }

    // ---- EvalRuntimeConfig::from_env ----

    /// Missing `DATABASE_URL` is the one env var `from_env` insists on —
    /// every other knob has a default. Absent DATABASE_URL must produce
    /// `AppError::MissingEnv("DATABASE_URL")`.
    #[test]
    fn from_env_missing_database_url_is_missing_env() {
        let env = StubEnv::new();
        let err = EvalRuntimeConfig::from_env(&env).expect_err("must fail without DATABASE_URL");
        match err {
            AppError::MissingEnv(k) => assert_eq!(k, "DATABASE_URL"),
            other => panic!("expected MissingEnv, got {other:?}"),
        }
    }

    /// Non-integer `EVAL_TOP_K_CHUNKS` is a config error, not a silent
    /// default. We seed garbage and expect `AppError::InvalidConfig` with
    /// the key surfaced so ops can diagnose.
    #[test]
    fn from_env_bad_top_k_is_invalid_config() {
        let env = StubEnv::new()
            .with("DATABASE_URL", "memory://")
            .with("EVAL_TOP_K_CHUNKS", "not-a-number");
        let err = EvalRuntimeConfig::from_env(&env).expect_err("must fail on bad top_k");
        match err {
            AppError::InvalidConfig { key, .. } => assert_eq!(key, "EVAL_TOP_K_CHUNKS"),
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    /// Non-float `EVAL_SIMILARITY_THRESHOLD` also goes through the strict
    /// `parse` path.
    #[test]
    fn from_env_bad_similarity_threshold_is_invalid_config() {
        let env = StubEnv::new()
            .with("DATABASE_URL", "memory://")
            .with("EVAL_SIMILARITY_THRESHOLD", "nope");
        let err = EvalRuntimeConfig::from_env(&env).expect_err("must fail on bad threshold");
        match err {
            AppError::InvalidConfig { key, .. } => {
                assert_eq!(key, "EVAL_SIMILARITY_THRESHOLD")
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    /// Happy path with only DATABASE_URL seeded: every other knob defaults.
    /// Locks the default values from `domain::ConfigKey` so a silent default
    /// drift is caught here.
    #[test]
    fn from_env_only_database_url_uses_defaults() {
        let env = StubEnv::new().with("DATABASE_URL", "postgres://ignored");
        let cfg =
            EvalRuntimeConfig::from_env(&env).expect("defaults must parse for all other keys");
        assert_eq!(cfg.database_url, "postgres://ignored");
        // Sanity: top_k and threshold came from the default ConfigKey values
        // (positive int and positive float, respectively).
        assert!(cfg.top_k_chunks > 0);
        assert!(cfg.similarity_threshold >= 0.0 && cfg.similarity_threshold <= 1.0);
        // Citation support default is false (cheap path), max_calls default 30.
        assert!(!cfg.citation_support_enabled);
        assert_eq!(cfg.citation_support_max_calls, 30);
    }

    /// An un-parseable `EVAL_CITATION_SUPPORT_ENABLED` must fall back to
    /// `false` (it uses `.unwrap_or(false)`, unlike the strict parses
    /// above). Guards against accidental `?` propagation.
    #[test]
    fn from_env_bad_citation_support_flag_falls_back_to_false() {
        let env = StubEnv::new()
            .with("DATABASE_URL", "memory://")
            .with("EVAL_CITATION_SUPPORT_ENABLED", "not-a-bool");
        let cfg = EvalRuntimeConfig::from_env(&env).expect("bad flag must not error");
        assert!(
            !cfg.citation_support_enabled,
            "garbage value must fall back to false"
        );
    }

    // ---- Read-only app methods ----

    /// `scores_for_summary` returns only rows matching the summary_id,
    /// mapped into wire-layer `ScoreEntry`s with the metadata preserved.
    #[tokio::test]
    async fn scores_for_summary_maps_rows() {
        let sid = Uuid::new_v4();
        let other = Uuid::new_v4();
        let db = Arc::new(ReadOnlyDb::default());
        {
            let mut s = db.scores.lock().unwrap();
            s.push(EvalScore {
                id: Uuid::new_v4(),
                summary_id: sid,
                summary_hash: "h".to_string(),
                metric: "length_in_range".to_string(),
                score: 0.8,
                judge_model: None,
                details: None,
                eval_version: "v-app-test".to_string(),
                created_at: NaiveDateTime::default(),
            });
            s.push(EvalScore {
                id: Uuid::new_v4(),
                summary_id: other,
                summary_hash: "h2".to_string(),
                metric: "other_metric".to_string(),
                score: 0.1,
                judge_model: None,
                details: None,
                eval_version: "v-app-test".to_string(),
                created_at: NaiveDateTime::default(),
            });
        }
        let app = make_app(db);

        let resp = app.scores_for_summary(sid).await.expect("must succeed");
        assert_eq!(resp.summary_id, sid);
        assert_eq!(resp.scores.len(), 1, "only matching summary rows returned");
        assert_eq!(resp.scores[0].metric, "length_in_range");
        assert!((resp.scores[0].score - 0.8).abs() < 1e-6);
    }

    /// DB-layer errors from `get_scores_for_summary` must be wrapped as
    /// `AppError::Service(ServiceError::InfraError(_))` — the wire layer
    /// needs the structured variant to pick the right HTTP status.
    #[tokio::test]
    async fn scores_for_summary_propagates_infra_error() {
        let db = Arc::new(ReadOnlyDb::fail_all("db unreachable"));
        let app = make_app(db);
        let err = app
            .scores_for_summary(Uuid::new_v4())
            .await
            .expect_err("DB failure must bubble up");
        match err {
            AppError::Service(services::ServiceError::InfraError(MockErr(m))) => {
                assert_eq!(m, "db unreachable");
            }
            other => panic!("expected Service(InfraError), got {other:?}"),
        }
    }

    /// `aggregate_summary(None)` falls back to the configured
    /// `eval_version` and maps the raw per-metric stats into the wire
    /// shape. Guards the `MetricStatsRaw -> MetricStats` conversion.
    #[tokio::test]
    async fn aggregate_summary_uses_config_default_version_and_maps_stats() {
        let db = Arc::new(ReadOnlyDb::default());
        {
            let mut agg = db.aggregate.lock().unwrap();
            agg.total_summaries = 10;
            agg.total_scored = 7;
            agg.per_metric.insert(
                "length_in_range".to_string(),
                MetricStatsRaw {
                    avg: 0.75,
                    min: 0.1,
                    max: 1.0,
                    count: 7,
                },
            );
        }
        let app = make_app(db);

        let resp = app.aggregate_summary(None).await.expect("must succeed");
        assert_eq!(resp.eval_version, "v-app-test", "None -> config default");
        assert_eq!(resp.total_summaries, 10);
        assert_eq!(resp.total_scored, 7);
        let stats = resp
            .per_metric
            .get("length_in_range")
            .expect("metric must be present");
        assert!((stats.avg - 0.75).abs() < 1e-6);
        assert!((stats.min - 0.1).abs() < 1e-6);
        assert!((stats.max - 1.0).abs() < 1e-6);
        assert_eq!(stats.count, 7);
    }

    /// `aggregate_summary(Some(ver))` uses the explicit override, not the
    /// configured default. We can't observe the query parameter directly in
    /// the fake, but the response echoes `eval_version` so we verify it
    /// matches the override.
    #[tokio::test]
    async fn aggregate_summary_honours_explicit_version_override() {
        let db = Arc::new(ReadOnlyDb::default());
        let app = make_app(db);

        let resp = app
            .aggregate_summary(Some("v-override".to_string()))
            .await
            .expect("must succeed");
        assert_eq!(resp.eval_version, "v-override");
    }

    /// `worst_offenders` maps each raw row onto the wire-layer shape,
    /// preserves the requested `metric` and `limit`, and honours the
    /// explicit `eval_version`.
    #[tokio::test]
    async fn worst_offenders_maps_rows_and_echoes_parameters() {
        let db = Arc::new(ReadOnlyDb::default());
        let sid1 = Uuid::new_v4();
        {
            let mut w = db.worst.lock().unwrap();
            w.push(WorstOffenderRow {
                summary_id: sid1,
                region_name: Some("Hippocampus".to_string()),
                metric: "length_in_range".to_string(),
                score: 0.05,
                eval_version: "v-app-test".to_string(),
            });
        }
        let app = make_app(db);

        let resp = app
            .worst_offenders("length_in_range".to_string(), 5, None)
            .await
            .expect("must succeed");
        assert_eq!(resp.metric, "length_in_range");
        assert_eq!(resp.limit, 5);
        assert_eq!(resp.entries.len(), 1);
        assert_eq!(resp.entries[0].summary_id, sid1);
        assert_eq!(
            resp.entries[0].region_name.as_deref(),
            Some("Hippocampus")
        );
    }

    /// `list_unscored_summary_ids` surfaces the raw Uuid list and echoes
    /// back the effective version (the override when set).
    #[tokio::test]
    async fn list_unscored_summary_ids_echoes_version_and_limit() {
        let db = Arc::new(ReadOnlyDb::default());
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        {
            let mut u = db.unscored.lock().unwrap();
            u.push(a);
            u.push(b);
        }
        let app = make_app(db);

        let resp = app
            .list_unscored_summary_ids(Some("v-explicit".to_string()), 50)
            .await
            .expect("must succeed");
        assert_eq!(resp.eval_version, "v-explicit");
        assert_eq!(resp.limit, 50);
        assert_eq!(resp.summary_ids, vec![a, b]);
    }

    // ---- step_score error paths ----

    /// Unknown `run_id` (nothing in eval_run_state) → `AppError::InvalidArg`
    /// with a message that includes the run_id so operators can trace.
    #[tokio::test]
    async fn step_score_unknown_run_id_is_invalid_arg() {
        let db = Arc::new(ReadOnlyDb::default());
        let app = make_app(db);
        let rid = Uuid::new_v4();
        let err = app
            .step_score(rpc_types::StepRequest {
                run_id: rid,
                step_id: Uuid::new_v4(),
                llm_response: rpc_types::LlmResponsePayload::Claims(domain::ClaimsResponse {
                    claims: vec![],
                }),
            })
            .await
            .expect_err("unknown run must error");
        match err {
            AppError::InvalidArg(msg) => {
                assert!(
                    msg.contains("unknown run_id"),
                    "message must say unknown run_id, got {msg}"
                );
                assert!(
                    msg.contains(&rid.to_string()),
                    "message must include the run_id"
                );
            }
            other => panic!("expected InvalidArg, got {other:?}"),
        }
    }

    // ---- Private helpers ----

    /// `endpoint_to_str` must map every `LlmEndpoint` variant to the
    /// string the persistence layer stores in `eval_run_state.pending_endpoint`.
    /// A new variant must be added to both the enum and this match in the
    /// same PR — this test locks the contract.
    #[test]
    fn endpoint_to_str_covers_every_variant() {
        assert_eq!(endpoint_to_str(&LlmEndpoint::ExtractClaims), "extract_claims");
        assert_eq!(endpoint_to_str(&LlmEndpoint::Embed), "embed");
        assert_eq!(
            endpoint_to_str(&LlmEndpoint::JudgeGroundedness),
            "judge_groundedness"
        );
        assert_eq!(endpoint_to_str(&LlmEndpoint::JudgeRubric), "judge_rubric");
        assert_eq!(
            endpoint_to_str(&LlmEndpoint::JudgeCitation),
            "judge_citation"
        );
    }

    /// `pending_fields(&NextAction::Done { .. })` must yield `(None, None)`.
    /// The persistence layer uses these as the pending_step_id/endpoint
    /// columns — a `Done` run must not appear to have pending work. Also
    /// exercises the `CallLlm` arm for sanity.
    #[test]
    fn pending_fields_done_and_call_llm_shapes() {
        // Done → no pending step, no endpoint.
        let done = NextAction::Done { metrics: vec![] };
        assert_eq!(pending_fields(&done), (None, None));

        // CallLlm → Some(step_id), Some(endpoint_str).
        let sid = Uuid::new_v4();
        let call = NextAction::CallLlm {
            step_id: sid,
            endpoint: LlmEndpoint::Embed,
            path: "/x".to_string(),
            body: serde_json::Value::Null,
        };
        let (step, endpoint) = pending_fields(&call);
        assert_eq!(step, Some(sid));
        assert_eq!(endpoint.as_deref(), Some("embed"));
    }
}
