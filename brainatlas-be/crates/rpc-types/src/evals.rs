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
}

// ---- /api/llm/judge-groundedness ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeGroundednessRequest {
    pub claim_text: String,
    pub evidence_chunks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
}

// ---- /api/llm/judge-rubric ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeRubricRequest {
    pub summary_text: String,
    pub region_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
}
