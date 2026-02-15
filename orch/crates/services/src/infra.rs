use uuid::Uuid;
use serde::de::DeserializeOwned;
use serde::Serialize;
use domain::{ConfigKey, RegionQuery, ProcessingBatch, BatchStatus};

pub trait EnvInfra: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error>;
}

#[derive(Debug, Clone)]
pub struct NewProcessedFetchTask {
    pub fetch_task_id: i64,
    pub region_id: Uuid,
    pub brainatlas_status: String,
}

#[derive(Debug, Clone)]
pub struct ProcessedFetchTask {
    pub fetch_task_id: i64,
    pub region_id: Uuid,
    pub processed_at: chrono::NaiveDateTime,
    pub brainatlas_status: String,
    pub brainatlas_started_at: Option<chrono::NaiveDateTime>,
    pub brainatlas_completed_at: Option<chrono::NaiveDateTime>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OrchConfig {
    pub key: String,
    pub value: String,
    pub description: Option<String>,
    pub updated_at: chrono::NaiveDateTime,
}

#[async_trait::async_trait]
pub trait OrchDatabase: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;
    
    /// Check if a fetch task has already been processed
    async fn get_processed_task(
        &self,
        database_url: &str,
        fetch_task_id: i64,
    ) -> Result<Option<ProcessedFetchTask>, Self::Error>;
    
    /// Insert a new processed task
    async fn insert_processed_task(
        &self,
        database_url: &str,
        task: NewProcessedFetchTask,
    ) -> Result<(), Self::Error>;
    
    /// Update brainatlas processing status
    async fn update_brainatlas_status(
        &self,
        database_url: &str,
        fetch_task_id: i64,
        status: &str,
        error: Option<String>,
    ) -> Result<(), Self::Error>;
    
    /// Get a configuration value
    async fn get_config(
        &self,
        database_url: &str,
        key: ConfigKey,
    ) -> Result<Option<String>, Self::Error>;
    
    /// Get all configuration
    async fn get_all_config(
        &self,
        database_url: &str,
    ) -> Result<Vec<OrchConfig>, Self::Error>;
    
    /// Update a configuration value
    async fn update_config(
        &self,
        database_url: &str,
        key: ConfigKey,
        value: &str,
    ) -> Result<(), Self::Error>;
}

#[async_trait::async_trait]
pub trait HttpClient: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;
    
    /// Make a GET request and deserialize the response
    async fn get<T: DeserializeOwned + Send>(&self, url: &str) -> Result<T, Self::Error>;
    
    /// Make a POST request with a JSON body and deserialize the response
    async fn post<Req: Serialize + Send + Sync, Res: DeserializeOwned + Send + Sync>(
        &self,
        url: &str,
        body: &Req,
    ) -> Result<Res, Self::Error>;
}

/// Batch management for region processing
#[async_trait::async_trait]
pub trait BatchManagement: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;
    
    /// Get queries for a region
    async fn get_queries(
        &self,
        database_url: &str,
        region_id: i32,
    ) -> Result<Vec<RegionQuery>, Self::Error>;
    
    /// Insert generated queries for a region
    async fn insert_queries(
        &self,
        database_url: &str,
        region_id: i32,
        queries: Vec<String>,
    ) -> Result<Vec<Uuid>, Self::Error>;
    
    /// Create a new processing batch
    async fn create_batch(
        &self,
        database_url: &str,
        region_id: i32,
        expected_count: i32,
    ) -> Result<Uuid, Self::Error>;
    
    /// Add fetch task IDs to a batch
    async fn add_tasks_to_batch(
        &self,
        database_url: &str,
        batch_id: Uuid,
        task_ids: Vec<i64>,
    ) -> Result<(), Self::Error>;
    
    /// Get batches by status
    async fn get_batches_by_status(
        &self,
        database_url: &str,
        status: BatchStatus,
    ) -> Result<Vec<ProcessingBatch>, Self::Error>;
    
    /// Update batch status
    async fn update_batch_status(
        &self,
        database_url: &str,
        batch_id: Uuid,
        status: BatchStatus,
        error: Option<String>,
    ) -> Result<(), Self::Error>;
    
    /// Mark batch complete with summary
    async fn complete_batch(
        &self,
        database_url: &str,
        batch_id: Uuid,
        summary_id: Uuid,
        content_hash: String,
    ) -> Result<(), Self::Error>;
    
    /// Get active batch for a region (if any)
    async fn get_active_batch(
        &self,
        database_url: &str,
        region_id: i32,
    ) -> Result<Option<ProcessingBatch>, Self::Error>;
}

/// Blanket: any `T: OrchDatabase + EnvInfra + HttpClient + BatchManagement` automatically satisfies `Infra`.
pub trait Infra:
   EnvInfra<Error = <Self as Infra>::Error>
   + OrchDatabase<Error = <Self as Infra>::Error>
   + HttpClient<Error = <Self as Infra>::Error>
   + BatchManagement<Error = <Self as Infra>::Error>
{
    type Error: std::error::Error + Send + Sync + 'static;
}

impl<E, T> Infra for T
where
    T: EnvInfra<Error = E> + OrchDatabase<Error = E> + HttpClient<Error = E> + BatchManagement<Error = E>,
    E: std::error::Error + Send + Sync + 'static,
{
    type Error = E;
}
