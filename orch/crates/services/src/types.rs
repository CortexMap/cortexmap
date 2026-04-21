use serde::{Deserialize, Serialize};

// Fetcher API types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDetailsResponse {
    pub found: bool,
    pub pmc_id: String,
    pub status: String,
    pub components: Vec<ComponentStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentStatus {
    pub component_type: String,
    pub status: String,
    pub s3_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskComponentsResponse {
    pub task_id: i64,
    pub pmc_id: String,
    pub task_status: String,
    pub components: Vec<ComponentStatus>,
}

// Brainatlas API types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UuidWrapper {
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperMetadataEntry {
    pub s3_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pmc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRegionRequest {
    pub region_id: UuidWrapper,
    pub batch_id: UuidWrapper,
    pub s3_keys: Vec<String>,
    /// Paper metadata for source attribution (s3_key -> pmc_id, uid, query)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paper_metadata: Vec<PaperMetadataEntry>,
    /// Chat model to use for summarization (e.g., "openai/gpt-4o-mini")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    /// Embedding model to use (e.g., "text-embedding-3-small")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    /// If true, only chunk and embed — skip RAG summarization.
    #[serde(default, skip_serializing_if = "is_false")]
    pub skip_summarization: bool,
    /// Opaque correlation id that brainatlas-be persists alongside every
    /// `llm_call_usage` row produced while processing this batch. Typical
    /// value is `batch:{batch_uuid}`. See `plans/2026-04-20-llm-cost-tracking-v1.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

fn is_false(v: &bool) -> bool {
    !v
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRegionResponse {
    pub region_id: UuidWrapper,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateQueriesRequest {
    pub region_name: String,
    pub count: u32,
    /// Correlation id for cost tracking; brainatlas-be persists this
    /// alongside the `llm_call_usage` row. Typical value is `region:{region_id}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateQueriesResponse {
    pub queries: Vec<String>,
}
