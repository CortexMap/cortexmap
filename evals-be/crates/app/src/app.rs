//! Top-level orchestration: wires services + infra together.
//!
//! The public entry points (`init_score`, `step_score`) drive the stateless
//! scoring state machine. Orch calls them from the outside, feeding LLM
//! responses back in and making the actual LLM HTTP calls on evals's behalf.

use crate::error::AppError;
use crate::run_eval::{probe_groundedness_cache, probe_rubric_cache, run_structural_metrics};
use domain::{compute_hash, EvalRunStatus};
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

        Ok(Self {
            database_url,
            eval_version,
            judge_chat_model,
            rubric_chat_model,
            embedding_model,
            top_k_chunks,
            similarity_threshold,
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

        let ctx = RunContext {
            summary: &summary,
            summary_hash: &summary_hash,
            eval_version: &eval_version,
            judge_chat_model: &self.config.judge_chat_model,
            rubric_chat_model: &self.config.rubric_chat_model,
            embedding_model: &self.config.embedding_model,
            top_k_chunks: self.config.top_k_chunks,
            similarity_threshold: self.config.similarity_threshold,
        };

        let (state, next) = state_machine::initial_action(&ctx, g_cached, r_cached, metrics.clone());

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
            .ok_or_else(|| {
                AppError::InvalidArg(format!("unknown run_id {}", req.run_id))
            })?;
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

                NextAction::Done { metrics: metrics_now }
            }
            NextAction::CallLlm {
                step_id,
                endpoint,
                path,
                body,
            } => {
                let endpoint_str = Some(endpoint_to_str(&endpoint).to_string());
                let state_json = serde_json::to_value(&new_state)
                    .expect("RunState serializable");
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
    }
}

fn pending_fields(next: &NextAction) -> (Option<Uuid>, Option<String>) {
    match next {
        NextAction::CallLlm { step_id, endpoint, .. } => {
            (Some(*step_id), Some(endpoint_to_str(endpoint).to_string()))
        }
        NextAction::Done { .. } => (None, None),
    }
}
