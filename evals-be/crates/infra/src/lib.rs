mod env;
mod error;
mod pg;
mod schema;

pub use env::*;
pub use error::*;
pub use pg::*;

use services::{EnvInfra, EvalsDatabase};
use std::sync::Arc;

/// Top-level infra struct: bundles Postgres + env into a single adapter the
/// app can share via `Arc`. No HTTP client here — evals-be is a pure state
/// machine as of 2026-04-19.
pub struct EvalsInfra {
    pub env: EvalsEnvInfra,
    pub pg: EvalsPostgresql,
}

impl EvalsInfra {
    pub fn new() -> Self {
        Self {
            env: EvalsEnvInfra::new(),
            pg: EvalsPostgresql::new(),
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

    async fn insert_run_state(
        &self,
        database_url: &str,
        summary_id: uuid::Uuid,
        eval_version: &str,
        state: &serde_json::Value,
        pending_step_id: Option<uuid::Uuid>,
        pending_endpoint: Option<&str>,
    ) -> Result<uuid::Uuid, Self::Error> {
        self.pg
            .insert_run_state(
                database_url,
                summary_id,
                eval_version,
                state,
                pending_step_id,
                pending_endpoint,
            )
            .await
    }

    async fn load_run_state(
        &self,
        database_url: &str,
        run_id: uuid::Uuid,
    ) -> Result<Option<services::LoadedRunState>, Self::Error> {
        self.pg.load_run_state(database_url, run_id).await
    }

    async fn save_run_state(
        &self,
        database_url: &str,
        run_id: uuid::Uuid,
        state: &serde_json::Value,
        pending_step_id: Option<uuid::Uuid>,
        pending_endpoint: Option<&str>,
    ) -> Result<(), Self::Error> {
        self.pg
            .save_run_state(database_url, run_id, state, pending_step_id, pending_endpoint)
            .await
    }

    async fn delete_run_state(
        &self,
        database_url: &str,
        run_id: uuid::Uuid,
    ) -> Result<(), Self::Error> {
        self.pg.delete_run_state(database_url, run_id).await
    }

    async fn delete_run_states_for_summary(
        &self,
        database_url: &str,
        summary_id: uuid::Uuid,
        eval_version: &str,
    ) -> Result<(), Self::Error> {
        self.pg
            .delete_run_states_for_summary(database_url, summary_id, eval_version)
            .await
    }
}

/// Convenience alias used by the server to spell the concrete `Arc<...>` type.
pub type SharedInfra = Arc<EvalsInfra>;
