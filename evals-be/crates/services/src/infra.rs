//! Infra-facing trait abstractions used by the eval app/service layer.
//!
//! Concrete implementations live in the `infra` crate:
//!  - `EvalsDatabase` — Postgres access for `eval_scores`, `eval_runs`,
//!    `eval_run_state`, plus read-only access to `region_summary` and
//!    `brain_region_embeddings`.
//!  - `EnvInfra` — env var lookup.
//!
//! As of 2026-04-19 evals-be is stateless w.r.t. outbound HTTP: the brainatlas
//! loop is driven externally by orch via `NextAction::CallLlm` envelopes in
//! the wire protocol, so there is no longer a `BrainatlasClient` trait.

use crate::ServiceError;
use async_trait::async_trait;
use domain::{EvalRun, EvalRunStatus, EvalScore, NewEvalScore};
use std::error::Error;
use uuid::Uuid;

// ---- Env ----

pub trait EnvInfra: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error>;
}

// ---- Read-only references to the brainatlas DB tables ----

#[derive(Debug, Clone)]
pub struct SummaryRow {
    pub id: Uuid,
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetrievedChunk {
    pub chunk_index: i32,
    pub chunk_text: String,
    /// Cosine similarity in [0, 1] (1 - cosine distance).
    pub similarity: f32,
}

/// Minimal row shape returned by `load_chunks_by_ids`. Carries just enough to
/// drive citation-correctness evals: the embedding's UUID (what the summary
/// cites), its owning `summary_id` (for scope checks), `chunk_index` (for
/// ordering), and the raw `chunk_text` (for the support judge prompt).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkRow {
    pub id: Uuid,
    pub summary_id: Uuid,
    pub chunk_index: i32,
    pub chunk_text: String,
}

// ---- Eval DB trait ----

/// Loaded state row for a run: (summary_id, eval_version, state_json, pending_step_id).
pub type LoadedRunState = (Uuid, String, serde_json::Value, Option<Uuid>);

#[async_trait]
pub trait EvalsDatabase: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    /// Look up the cached score row by `(summary_hash, metric, eval_version)`.
    async fn lookup_score_by_hash(
        &self,
        database_url: &str,
        summary_hash: &str,
        metric: &str,
        eval_version: &str,
    ) -> Result<Option<EvalScore>, Self::Error>;

    /// Insert a new score row, then re-select to resolve concurrent writers
    /// to the same row (`INSERT ... ON CONFLICT (summary_hash, metric, eval_version) DO NOTHING`).
    async fn insert_score(
        &self,
        database_url: &str,
        new: NewEvalScore,
    ) -> Result<EvalScore, Self::Error>;

    /// Load summary row + region details by summary id.
    async fn get_summary(
        &self,
        database_url: &str,
        summary_id: Uuid,
    ) -> Result<Option<SummaryRow>, Self::Error>;

    /// All scores for a single summary (for `GET /scores/:summary_id`).
    async fn get_scores_for_summary(
        &self,
        database_url: &str,
        summary_id: Uuid,
    ) -> Result<Vec<EvalScore>, Self::Error>;

    /// Aggregate counts + per-metric stats for `/api/evals/summary`.
    async fn get_eval_aggregate(
        &self,
        database_url: &str,
        eval_version: &str,
    ) -> Result<EvalAggregate, Self::Error>;

    /// `N` lowest-scoring summaries for one metric, joined with region name.
    async fn get_worst_offenders(
        &self,
        database_url: &str,
        metric: &str,
        eval_version: &str,
        limit: i64,
    ) -> Result<Vec<WorstOffenderRow>, Self::Error>;

    /// Upsert an `eval_runs` row for `(summary_id, eval_version)`.
    async fn upsert_run(
        &self,
        database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
        status: EvalRunStatus,
        error_message: Option<String>,
    ) -> Result<EvalRun, Self::Error>;

    /// Active summaries that have no `complete` run for the current
    /// `eval_version`. Used by orch to find work.
    async fn list_unscored_summary_ids(
        &self,
        database_url: &str,
        eval_version: &str,
        limit: i64,
    ) -> Result<Vec<Uuid>, Self::Error>;

    /// pgvector similarity query against `brain_region_embeddings`,
    /// scoped to `summary_id` so claims must be grounded in *this* summary's
    /// source chunks.
    async fn retrieve_chunks_for_summary(
        &self,
        database_url: &str,
        summary_id: Uuid,
        embedding: &[f32],
        top_k: i64,
        min_similarity: f32,
    ) -> Result<Vec<RetrievedChunk>, Self::Error>;

    /// Batch lookup of `brain_region_embeddings` rows by primary key. Returns
    /// only rows that exist — any requested UUIDs not found in the table are
    /// silently omitted, which is how "orphan" citations are detected in
    /// `citations::citation_validity_score`.
    ///
    /// An empty `chunk_ids` input must return `Ok(vec![])` without touching
    /// the database.
    async fn load_chunks_by_ids(
        &self,
        database_url: &str,
        chunk_ids: &[Uuid],
    ) -> Result<Vec<ChunkRow>, Self::Error>;

    // ---- eval_run_state (state machine persistence) ----

    /// Insert a fresh run-state row. Returns the generated `run_id`. The row
    /// is expected to hold the next pending step id + endpoint.
    async fn insert_run_state(
        &self,
        database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
        state: &serde_json::Value,
        pending_step_id: Option<Uuid>,
        pending_endpoint: Option<&str>,
    ) -> Result<Uuid, Self::Error>;

    /// Load a run state by id. Returns `None` if no row exists.
    async fn load_run_state(
        &self,
        database_url: &str,
        run_id: Uuid,
    ) -> Result<Option<LoadedRunState>, Self::Error>;

    /// Rewrite the state + pending step for an existing run.
    async fn save_run_state(
        &self,
        database_url: &str,
        run_id: Uuid,
        state: &serde_json::Value,
        pending_step_id: Option<Uuid>,
        pending_endpoint: Option<&str>,
    ) -> Result<(), Self::Error>;

    /// Delete a run-state row (called on Done, or on /init re-entry for the
    /// same `summary_id` to clean up an abandoned run).
    async fn delete_run_state(&self, database_url: &str, run_id: Uuid) -> Result<(), Self::Error>;

    /// Remove every stale `eval_run_state` row for the given
    /// `(summary_id, eval_version)` pair. Called on `/init` re-entry so
    /// abandoned runs don't leak forever.
    async fn delete_run_states_for_summary(
        &self,
        database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, Default)]
pub struct EvalAggregate {
    pub total_summaries: i64,
    pub total_scored: i64,
    pub per_metric: std::collections::HashMap<String, MetricStatsRaw>,
}

#[derive(Debug, Clone, Default)]
pub struct MetricStatsRaw {
    pub avg: f32,
    pub min: f32,
    pub max: f32,
    pub count: i64,
}

#[derive(Debug, Clone)]
pub struct WorstOffenderRow {
    pub summary_id: Uuid,
    pub region_name: Option<String>,
    pub metric: String,
    pub score: f32,
    pub eval_version: String,
}

// ---- Convenience: convert any infra error into a ServiceError ----

pub fn into_svc_err<E: Error + Send + Sync + 'static>(e: E) -> ServiceError<E> {
    ServiceError::InfraError(e)
}
