use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
pub struct ProcessRegionRequest {
    pub region_id: String,
    pub s3_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRegionResponse {
    pub region_id: String,
    pub detail: String,
}
