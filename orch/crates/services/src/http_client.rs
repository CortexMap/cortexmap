use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum HttpClientError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Service returned error: {status} - {message}")]
    ServiceError { status: u16, message: String },
}

pub struct HttpClient {
    client: Client,
}

impl HttpClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Get completed tasks from fetcher
    pub async fn get_completed_tasks(
        &self,
        fetcher_url: &str,
        limit: i32,
    ) -> Result<Vec<FetchTask>, HttpClientError> {
        let url = format!("{}/api/queue/tasks?status=completed&limit={}", fetcher_url, limit);
        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            return Err(HttpClientError::ServiceError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }
        
        Ok(response.json().await?)
    }

    /// Get task components (S3 keys) from fetcher
    pub async fn get_task_components(
        &self,
        fetcher_url: &str,
        task_id: i64,
    ) -> Result<TaskComponents, HttpClientError> {
        let url = format!("{}/api/queue/task/{}/components", fetcher_url, task_id);
        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            return Err(HttpClientError::ServiceError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }
        
        Ok(response.json().await?)
    }

    /// Call brainatlas to process a region
    pub async fn process_region(
        &self,
        brainatlas_url: &str,
        region_id: Uuid,
        s3_keys: Vec<String>,
    ) -> Result<ProcessRegionResponse, HttpClientError> {
        let url = format!("{}/api/process", brainatlas_url);
        let request = ProcessRegionRequest {
            region_id: Some(RegionId { value: region_id.to_string() }),
            s3_keys,
        };
        
        let response = self.client.post(&url).json(&request).send().await?;
        
        if !response.status().is_success() {
            return Err(HttpClientError::ServiceError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }
        
        Ok(response.json().await?)
    }
}

// DTOs matching fetcher/brainatlas APIs

#[derive(Debug, Clone, Deserialize)]
pub struct FetchTask {
    pub found: bool,
    pub pmc_id: String,
    pub status: String,
    pub query: Option<String>,
    pub priority: Option<i32>,
    pub created_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub worker_id: Option<String>,
    pub worker_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskComponents {
    pub task_id: i64,
    pub pmc_id: String,
    pub task_status: String,
    pub components: Vec<ComponentStatus>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentStatus {
    pub component_type: String,
    pub status: String,
    pub s3_key: Option<String>,
    pub attempt_count: i32,
    pub max_attempts: i32,
    pub error_message: Option<String>,
    pub last_attempted_at: Option<i64>,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessRegionRequest {
    pub region_id: Option<RegionId>,
    pub s3_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionId {
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessRegionResponse {
    pub region_id: Option<RegionId>,
    pub detail: String,
}
