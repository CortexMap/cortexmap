//! HTTP wire types for stateless eval-related endpoints exposed by brainatlas-be.
//!
//! These are plain serde structs (no protobuf) because callers are HTTP/JSON
//! clients (`evals-be`) rather than gRPC consumers. Keeping them in `rpc-types`
//! lets both server and any Rust client share the contract.

use serde::{Deserialize, Serialize};

// ---- /api/llm/embed ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedRequest {
    pub text: String,
    /// Optional override of the embedding model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    /// Opaque caller-supplied ID used to attribute the LLM cost back to the
    /// originating eval run/step or region summary. See the cost tracking
    /// design in `plans/2026-04-20-llm-cost-tracking-v1.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedResponse {
    pub embedding: Vec<f32>,
}

// ---- /api/llm/extract-claims ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractClaimsRequest {
    pub summary_text: String,
    pub region_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

// ---- /api/llm/judge-groundedness ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeGroundednessRequest {
    pub claim_text: String,
    pub evidence_chunks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

// ---- /api/llm/judge-rubric ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeRubricRequest {
    pub summary_text: String,
    pub region_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

// ---- /api/llm/usage ----
//
// Aggregate view of the `llm_call_usage` table. Query string parameters map
// 1:1 to `domain::UsageAggregateFilter`; unset parameters are not applied.

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageAggregateQuery {
    /// Inclusive lower-bound on `created_at`, RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Inclusive upper-bound on `created_at`, RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Prefix match on `correlation_id`, e.g. `eval:{run_id}:` to aggregate
    /// all steps of an eval run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_tag: Option<String>,
}

// ---- /api/llm/judge-citation ----
//
// Stateless "did the author cite the right chunk?" judge. Distinct from
// `judge-groundedness`: the caller passes exactly ONE chunk (the one the
// author cited for this claim) plus the enclosing sentence as context.
//
// Response reuses `GroundednessVerdict` (from brainatlas-be domain) so
// wire shape stays uniform. `supporting_chunks` is always empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeCitationRequest {
    pub claim_text: String,
    pub sentence_context: String,
    pub chunk_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}
