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
pub struct ProcessRegionRequest {
    pub region_id: UuidWrapper,
    pub batch_id: UuidWrapper,
    pub s3_keys: Vec<String>,
    /// Chat model to use for summarization (e.g., "openai/gpt-4o-mini")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    /// Embedding model to use (e.g., "text-embedding-3-small")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateQueriesResponse {
    pub queries: Vec<String>,
}
