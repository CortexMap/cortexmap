mod env;
mod error;
mod http;
mod pg;
mod schema;

pub use env::*;
pub use error::*;
pub use http::*;
pub use pg::*;

use services::{BrainatlasClient, EnvInfra, EvalsDatabase};
use std::sync::Arc;

/// Top-level infra struct: bundles all infra-side adapters into one type so
/// the server can hand a single `Arc<EvalsInfra>` to the app.
pub struct EvalsInfra {
    pub env: EvalsEnvInfra,
    pub pg: EvalsPostgresql,
    pub http: EvalsHttpClient,
}

impl EvalsInfra {
    pub fn new() -> Self {
        Self {
            env: EvalsEnvInfra::new(),
            pg: EvalsPostgresql::new(),
            http: EvalsHttpClient::new(),
        }
    }
}

impl Default for EvalsInfra {
    fn default() -> Self {
        Self::new()
    }
}

// Forward env trait
impl EnvInfra for EvalsInfra {
    type Error = InfraError;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
        self.env.get_env_var(key)
    }
}

// Forward DB trait
#[async_trait::async_trait]
impl EvalsDatabase for EvalsInfra {
    type Error = InfraError;

    async fn lookup_score_by_hash(
        &self,
        database_url: &str,
        summary_hash: &str,
        metric: &str,
        eval_version: &str,
    ) -> Result<Option<domain::EvalScore>, Self::Error> {
        self.pg
            .lookup_score_by_hash(database_url, summary_hash, metric, eval_version)
            .await
    }

    async fn insert_score(
        &self,
        database_url: &str,
        new: domain::NewEvalScore,
    ) -> Result<domain::EvalScore, Self::Error> {
        self.pg.insert_score(database_url, new).await
    }

    async fn get_summary(
        &self,
        database_url: &str,
        summary_id: uuid::Uuid,
    ) -> Result<Option<services::SummaryRow>, Self::Error> {
        self.pg.get_summary(database_url, summary_id).await
    }

    async fn get_scores_for_summary(
        &self,
        database_url: &str,
        summary_id: uuid::Uuid,
    ) -> Result<Vec<domain::EvalScore>, Self::Error> {
        self.pg.get_scores_for_summary(database_url, summary_id).await
    }

    async fn get_eval_aggregate(
        &self,
        database_url: &str,
        eval_version: &str,
    ) -> Result<services::EvalAggregate, Self::Error> {
        self.pg.get_eval_aggregate(database_url, eval_version).await
    }

    async fn get_worst_offenders(
        &self,
        database_url: &str,
        metric: &str,
        eval_version: &str,
        limit: i64,
    ) -> Result<Vec<services::WorstOffenderRow>, Self::Error> {
        self.pg
            .get_worst_offenders(database_url, metric, eval_version, limit)
            .await
    }

    async fn upsert_run(
        &self,
        database_url: &str,
        summary_id: uuid::Uuid,
        eval_version: &str,
        status: domain::EvalRunStatus,
        error_message: Option<String>,
    ) -> Result<domain::EvalRun, Self::Error> {
        self.pg
            .upsert_run(database_url, summary_id, eval_version, status, error_message)
            .await
    }

    async fn list_unscored_summary_ids(
        &self,
        database_url: &str,
        eval_version: &str,
        limit: i64,
    ) -> Result<Vec<uuid::Uuid>, Self::Error> {
        self.pg
            .list_unscored_summary_ids(database_url, eval_version, limit)
            .await
    }

    async fn retrieve_chunks_for_summary(
        &self,
        database_url: &str,
        summary_id: uuid::Uuid,
        embedding: &[f32],
        top_k: i64,
        min_similarity: f32,
    ) -> Result<Vec<services::RetrievedChunk>, Self::Error> {
        self.pg
            .retrieve_chunks_for_summary(database_url, summary_id, embedding, top_k, min_similarity)
            .await
    }
}

#[async_trait::async_trait]
impl BrainatlasClient for EvalsInfra {
    type Error = InfraError;

    async fn extract_claims(
        &self,
        base_url: &str,
        req: brainatlas_rpc_types::evals::ExtractClaimsRequest,
    ) -> Result<domain::ClaimsResponse, Self::Error> {
        self.http.extract_claims(base_url, req).await
    }

    async fn embed(
        &self,
        base_url: &str,
        req: brainatlas_rpc_types::evals::EmbedRequest,
    ) -> Result<brainatlas_rpc_types::evals::EmbedResponse, Self::Error> {
        self.http.embed(base_url, req).await
    }

    async fn judge_groundedness(
        &self,
        base_url: &str,
        req: brainatlas_rpc_types::evals::JudgeGroundednessRequest,
    ) -> Result<domain::GroundednessVerdict, Self::Error> {
        self.http.judge_groundedness(base_url, req).await
    }

    async fn judge_rubric(
        &self,
        base_url: &str,
        req: brainatlas_rpc_types::evals::JudgeRubricRequest,
    ) -> Result<domain::RubricScores, Self::Error> {
        self.http.judge_rubric(base_url, req).await
    }

    async fn check_health(&self, base_url: &str) -> Result<(), Self::Error> {
        self.http.check_health(base_url).await
    }
}

/// Convenience alias used by the server to spell the concrete `Arc<...>` type.
pub type SharedInfra = Arc<EvalsInfra>;
