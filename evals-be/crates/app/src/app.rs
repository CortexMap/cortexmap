//! Top-level orchestration: wires services + infra together. Public entry
//! points used by the HTTP server live here.

use crate::error::AppError;
use crate::run_eval::{run_all_metrics, MetricOutcome};
use rpc_types::{
    EvalSummaryResponse, MetricResult, MetricStats, ScoreEntry, ScoreResponse,
    ScoresForSummaryResponse, UnscoredResponse, WorstOffender, WorstOffendersResponse,
};
use services::{BrainatlasClient, EnvInfra, EvalsDatabase};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

/// All knobs needed to score a summary. Loaded once at startup from env;
/// per-call overrides happen via the request body's `eval_version` field.
#[derive(Debug, Clone)]
pub struct EvalRuntimeConfig {
    pub database_url: String,
    pub brainatlas_base_url: String,
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
            env.get_env_var(key)
                .unwrap_or_else(|_| default.to_string())
        }

        let database_url = env
            .get_env_var("DATABASE_URL")
            .map_err(|_| AppError::MissingEnv("DATABASE_URL".to_string()))?;

        let brainatlas_base_url = get_or_default(
            env,
            "BRAINATLAS_BASE_URL",
            ConfigKey::BrainatlasBaseUrl.default_value(),
        );

        let eval_version = get_or_default(
            env,
            "EVAL_VERSION",
            ConfigKey::EvalVersion.default_value(),
        );

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
            brainatlas_base_url,
            eval_version,
            judge_chat_model,
            rubric_chat_model,
            embedding_model,
            top_k_chunks,
            similarity_threshold,
        })
    }
}

/// Concrete app composed of database + brainatlas client + env. Generic over
/// the infra error type so tests can swap mocks in.
pub struct EvalsApp<DB, BC, EN, E>
where
    DB: EvalsDatabase<Error = E>,
    BC: BrainatlasClient<Error = E>,
    EN: EnvInfra<Error = E>,
    E: Error + Send + Sync + 'static,
{
    pub db: Arc<DB>,
    pub brainatlas: Arc<BC>,
    pub env: Arc<EN>,
    pub config: EvalRuntimeConfig,
}

impl<DB, BC, EN, E> EvalsApp<DB, BC, EN, E>
where
    DB: EvalsDatabase<Error = E>,
    BC: BrainatlasClient<Error = E>,
    EN: EnvInfra<Error = E>,
    E: Error + Send + Sync + 'static,
{
    pub fn new(db: Arc<DB>, brainatlas: Arc<BC>, env: Arc<EN>) -> Result<Self, AppError<E>> {
        let config = EvalRuntimeConfig::from_env(env.as_ref())?;
        Ok(Self {
            db,
            brainatlas,
            env,
            config,
        })
    }

    /// Score a single summary across all metrics. Returns the per-metric
    /// outcome including a `cached` flag so callers can observe cache
    /// effectiveness.
    pub async fn score_summary(
        &self,
        summary_id: Uuid,
        eval_version_override: Option<String>,
    ) -> Result<ScoreResponse, AppError<E>> {
        let eval_version = eval_version_override.unwrap_or_else(|| self.config.eval_version.clone());

        let outcomes = run_all_metrics(
            self.db.as_ref(),
            self.brainatlas.as_ref(),
            &self.config,
            summary_id,
            &eval_version,
        )
        .await?;

        let metrics = outcomes.into_iter().map(metric_outcome_to_result).collect();

        Ok(ScoreResponse {
            summary_id,
            eval_version,
            metrics,
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

    /// HTTP health check on the dependent brainatlas service.
    pub async fn brainatlas_health(&self) -> Result<(), AppError<E>> {
        self.brainatlas
            .check_health(&self.config.brainatlas_base_url)
            .await
            .map_err(services::ServiceError::InfraError)?;
        Ok(())
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

fn metric_outcome_to_result(o: MetricOutcome) -> MetricResult {
    MetricResult {
        metric: o.metric,
        score: o.score,
        cached: o.cached,
        judge_model: o.judge_model,
    }
}
