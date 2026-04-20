//! HTTP wire types for evals-be's public API.
//!
//! evals-be is a pure state-machine: orch drives an `init`/`step` loop that
//! makes all outbound LLM calls on behalf of evals. Every `NextAction::CallLlm`
//! carries the exact body orch should POST to brainatlas, plus an opaque
//! `step_id` the next `/step` request must echo back for idempotency.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---- POST /api/evals/score/init ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitScoreRequest {
    pub summary_id: Uuid,
    /// Optional override of the eval_version. Defaults to `ConfigKey::EvalVersion`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitScoreResponse {
    pub run_id: Uuid,
    pub summary_id: Uuid,
    pub eval_version: String,
    pub next: NextAction,
}

// ---- POST /api/evals/score/step ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRequest {
    pub run_id: Uuid,
    /// Echoes the `step_id` returned on the previous `CallLlm` action.
    pub step_id: Uuid,
    pub llm_response: LlmResponsePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResponse {
    pub run_id: Uuid,
    pub next: NextAction,
}

// ---- State machine action envelope ----

/// What orch should do next. Either make an LLM call on evals's behalf, or
/// stop because the run is complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NextAction {
    CallLlm {
        /// Unique per outstanding step. Must be echoed back in the next
        /// `StepRequest`.
        step_id: Uuid,
        endpoint: LlmEndpoint,
        /// The path orch should POST to (relative to the brainatlas base URL).
        path: String,
        /// The exact JSON body orch should POST. Shape depends on `endpoint`.
        body: serde_json::Value,
    },
    Done {
        metrics: Vec<MetricResult>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmEndpoint {
    ExtractClaims,
    Embed,
    JudgeGroundedness,
    JudgeRubric,
}

impl LlmEndpoint {
    /// Path relative to the brainatlas base URL.
    pub fn path(self) -> &'static str {
        match self {
            LlmEndpoint::ExtractClaims => "/brainatlas-be/api/llm/extract-claims",
            LlmEndpoint::Embed => "/brainatlas-be/api/llm/embed",
            LlmEndpoint::JudgeGroundedness => "/brainatlas-be/api/llm/judge-groundedness",
            LlmEndpoint::JudgeRubric => "/brainatlas-be/api/llm/judge-rubric",
        }
    }
}

/// One of the four possible LLM response shapes orch can feed back to evals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmResponsePayload {
    Claims(domain::ClaimsResponse),
    Embed(brainatlas_rpc_types::evals::EmbedResponse),
    Groundedness(domain::GroundednessVerdict),
    Rubric(domain::RubricScores),
}

// ---- MetricResult (unchanged) ----

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
