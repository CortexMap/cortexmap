use domain::{ConfigEntry, ConfigEntryUpdate, ConfigKey, PendingTask, PollResult, ProcessResult, ProcessingBatch, RegionQuery, RegionSummary};
use std::error::Error;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait CompletionOrchestrator: Send + Sync {
    type Error: Error + Send + Sync;

    /// Poll for completed fetch tasks that need LLM processing
    /// Returns tasks that haven't been processed yet
    async fn poll(&self) -> Result<PollResult, Self::Error>;

    /// Process a list of pending tasks
    /// Calls brainatlas API to chunk/embed/summarize each task
    async fn process(&self, tasks: Vec<PendingTask>) -> Result<ProcessResult, Self::Error>;
    
    /// Get a configuration value by key
    async fn get_config(&self, key: ConfigKey) -> Result<Option<String>, Self::Error>;
}

/// Trait for managing brain regions and their summaries
#[async_trait::async_trait]
pub trait RegionManagement: Send + Sync {
    type Error: Error + Send + Sync;
    
    /// Get summaries for a region from brainatlas
    async fn get_summaries(&self, region_id: Uuid) -> Result<Vec<RegionSummary>, Self::Error>;
    
    /// Get active processing batch for a region
    async fn get_active_batch(&self, region_id: Uuid) -> Result<Option<ProcessingBatch>, Self::Error>;
    
    /// Get stored queries for a region
    async fn get_queries(&self, region_id: Uuid) -> Result<Vec<RegionQuery>, Self::Error>;
    
    /// Store generated queries for a region
    async fn store_queries(&self, region_id: Uuid, queries: Vec<String>) -> Result<Vec<Uuid>, Self::Error>;
    
    /// Generate search queries for a region using LLM
    async fn generate_queries(&self, region_name: &str, count: u32) -> Result<Vec<String>, Self::Error>;
    
    /// Update batch status (for invalidation)
    async fn update_batch_status(&self, batch_id: Uuid, status: domain::BatchStatus, error: Option<String>) -> Result<(), Self::Error>;
    
    /// Get batches by status (for stats)
    async fn get_batches_by_status(&self, status: domain::BatchStatus) -> Result<Vec<domain::ProcessingBatch>, Self::Error>;
    
    /// Get region name from region_mapping
    async fn get_region_name(&self, region_id: Uuid) -> Result<String, Self::Error>;
    
    /// Get total number of regions in region_mapping
    async fn get_total_regions(&self) -> Result<i64, Self::Error>;
    
    /// Count regions that have no batches
    async fn count_regions_without_batches(&self) -> Result<i64, Self::Error>;
    
    /// Get query generation limit from config (or default)
    async fn get_query_generation_limit(&self) -> Result<Option<u32>, Self::Error>;
}

/// Trait for managing batches and fetcher integration
#[async_trait::async_trait]
pub trait BatchOrchestration: Send + Sync {
    type Error: Error + Send + Sync;
    
    /// Create a new processing batch
    async fn create_batch(&self, region_id: Uuid, expected_count: usize) -> Result<Uuid, Self::Error>;
    
    /// Enqueue a fetch task in the fetcher service
    async fn enqueue_fetch_task(&self, query: String, region_id: Uuid, priority: i32) -> Result<i64, Self::Error>;
    
    /// Add task IDs to a batch
    async fn add_tasks_to_batch(&self, batch_id: Uuid, task_ids: Vec<i64>) -> Result<(), Self::Error>;
}

/// Trait for configuration management
#[async_trait::async_trait]
pub trait ConfigManagement: Send + Sync {
    type Error: Error + Send + Sync;
    
    /// Get all configuration entries
    async fn get_all_config(&self) -> Result<Vec<ConfigEntry>, Self::Error>;
    
    /// Update configuration entries
    async fn update_config(&self, entries: Vec<ConfigEntryUpdate>) -> Result<Vec<ConfigEntry>, Self::Error>;
}

pub trait Services: 
    CompletionOrchestrator<Error = <Self as Services>::Error>
    + RegionManagement<Error = <Self as Services>::Error>
    + BatchOrchestration<Error = <Self as Services>::Error>
    + ConfigManagement<Error = <Self as Services>::Error>
{
    type Error: Error + Send + Sync;
}

impl<E, T> Services for T
where
    T: CompletionOrchestrator<Error = E>
        + RegionManagement<Error = E>
        + BatchOrchestration<Error = E>
        + ConfigManagement<Error = E>,
    E: Error + Send + Sync,
{
    type Error = E;
}
