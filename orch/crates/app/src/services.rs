use domain::{ConfigKey, PendingTask, PollResult, ProcessResult, ProcessingBatch, RegionQuery, RegionSummary};
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
    async fn get_summaries(&self, region_id: i32) -> Result<Vec<RegionSummary>, Self::Error>;
    
    /// Get active processing batch for a region
    async fn get_active_batch(&self, region_id: i32) -> Result<Option<ProcessingBatch>, Self::Error>;
    
    /// Get stored queries for a region
    async fn get_queries(&self, region_id: i32) -> Result<Vec<RegionQuery>, Self::Error>;
    
    /// Store generated queries for a region
    async fn store_queries(&self, region_id: i32, queries: Vec<String>) -> Result<Vec<Uuid>, Self::Error>;
    
    /// Generate search queries for a region using LLM
    async fn generate_queries(&self, region_name: &str, count: u32) -> Result<Vec<String>, Self::Error>;
}

/// Trait for managing batches and fetcher integration
#[async_trait::async_trait]
pub trait BatchOrchestration: Send + Sync {
    type Error: Error + Send + Sync;
    
    /// Create a new processing batch
    async fn create_batch(&self, region_id: i32, expected_count: usize) -> Result<Uuid, Self::Error>;
    
    /// Enqueue a fetch task in the fetcher service
    async fn enqueue_fetch_task(&self, query: String, region_id: i32, priority: i32) -> Result<i64, Self::Error>;
    
    /// Add task IDs to a batch
    async fn add_tasks_to_batch(&self, batch_id: Uuid, task_ids: Vec<i64>) -> Result<(), Self::Error>;
}

pub trait Services: 
    CompletionOrchestrator<Error = <Self as Services>::Error>
    + RegionManagement<Error = <Self as Services>::Error>
    + BatchOrchestration<Error = <Self as Services>::Error>
{
    type Error: Error + Send + Sync;
}

impl<E, T> Services for T
where
    T: CompletionOrchestrator<Error = E>
        + RegionManagement<Error = E>
        + BatchOrchestration<Error = E>,
    E: Error + Send + Sync,
{
    type Error = E;
}
