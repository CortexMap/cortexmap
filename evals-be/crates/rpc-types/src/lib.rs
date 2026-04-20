//! HTTP wire types for evals-be's public API.
//!
//! These are plain JSON contracts (no protobuf). The cache is observable to
//! callers via the `cached: bool` field on each `MetricResult`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---- POST /api/evals/score ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreRequest {
    pub summary_id: Uuid,
    /// Optional override of the eval_version. Defaults to `ConfigKey::EvalVersion`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreResponse {
    pub summary_id: Uuid,
    pub eval_version: String,
    pub metrics: Vec<MetricResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricResult {
    pub metric: String,
    pub score: f32,
    /// `true` if the score was returned from the cache (no compute / LLM call).
    pub cached: bool,
    pub judge_model: Option<String>,
}

// ---- GET /api/evals/scores/:summary_id ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoresForSummaryResponse {
    pub summary_id: Uuid,
    pub scores: Vec<ScoreEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreEntry {
    pub metric: String,
    pub score: f32,
    pub eval_version: String,
    pub judge_model: Option<String>,
    pub details: Option<serde_json::Value>,
    pub created_at: String,
}

// ---- GET /api/evals/summary?eval_version=... ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalSummaryResponse {
    pub eval_version: String,
    pub total_summaries: i64,
    pub total_scored: i64,
    pub per_metric: HashMap<String, MetricStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricStats {
    pub avg: f32,
    pub min: f32,
    pub max: f32,
    pub count: i64,
}

// ---- GET /api/evals/worst?metric=...&limit=... ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorstOffender {
    pub summary_id: Uuid,
    pub region_name: Option<String>,
    pub metric: String,
    pub score: f32,
    pub eval_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorstOffendersResponse {
    pub metric: String,
    pub limit: i64,
    pub entries: Vec<WorstOffender>,
}

// ---- GET /api/evals/unscored?eval_version=...&limit=... ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnscoredResponse {
    pub eval_version: String,
    pub limit: i64,
    pub summary_ids: Vec<Uuid>,
}
