use domain::{
    AllocateWorkersRequest, ChunkSourceResponse, ConfigEntry, ConfigEntryUpdate, ConfigKey,
    PendingTask, PollResult, ProcessResult, ProcessingBatch, RegionQuery, RegionSummary,
    StopWorkersRequest, WorkerAllocationResponse, WorkerStatus, WorkerStopResponse,
};
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
    async fn get_active_batch(
        &self,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error>;

    /// Get most recent batch for a region (regardless of status)
    async fn get_recent_batch(
        &self,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error>;

    /// Get stored queries for a region
    async fn get_queries(&self, region_id: Uuid) -> Result<Vec<RegionQuery>, Self::Error>;

    /// Store generated queries for a region
    async fn store_queries(
        &self,
        region_id: Uuid,
        queries: Vec<String>,
    ) -> Result<Vec<Uuid>, Self::Error>;

    /// Generate search queries for a region using LLM
    async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, Self::Error>;

    /// Update batch status (for invalidation)
    async fn update_batch_status(
        &self,
        batch_id: Uuid,
        status: domain::BatchStatus,
        error: Option<String>,
    ) -> Result<(), Self::Error>;

    /// Get batches by status (for stats)
    async fn get_batches_by_status(
        &self,
        status: domain::BatchStatus,
    ) -> Result<Vec<domain::ProcessingBatch>, Self::Error>;

    /// Get region name from region_mapping
    async fn get_region_name(&self, region_id: Uuid) -> Result<String, Self::Error>;

    /// Get total number of regions in region_mapping
    async fn get_total_regions(&self) -> Result<i64, Self::Error>;

    /// Count regions that have no batches
    async fn count_regions_without_batches(&self) -> Result<i64, Self::Error>;

    /// Count collecting batches with at least one in_progress fetch task
    async fn count_actively_fetching_regions(&self) -> Result<i64, Self::Error>;

    /// Get query generation limit from config (or default)
    async fn get_query_generation_limit(&self) -> Result<Option<u32>, Self::Error>;

    /// Get all regions from region_mapping
    async fn get_all_regions(&self) -> Result<Vec<domain::Region>, Self::Error>;

    /// Delete all queries for a region (used for invalidation)
    async fn delete_queries(&self, region_id: Uuid) -> Result<(), Self::Error>;

    /// Resolve a chunk UUID to its full source details via brainatlas-be
    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<ChunkSourceResponse, Self::Error>;

    /// Search regions by natural language query across names, acronyms, and latest summaries
    async fn reverse_search(&self, query: &str) -> Result<domain::SearchResponse, Self::Error>;
}

/// Trait for managing batches and fetcher integration
#[async_trait::async_trait]
pub trait BatchOrchestration: Send + Sync {
    type Error: Error + Send + Sync;

    /// Create a new processing batch
    async fn create_batch(
        &self,
        region_id: Uuid,
        expected_count: usize,
    ) -> Result<Uuid, Self::Error>;

    /// Enqueue a fetch task in the fetcher service
    /// Returns the list of task IDs created (one query may result in multiple papers/tasks)
    async fn enqueue_fetch_task(
        &self,
        query: String,
        region_id: Uuid,
        priority: i32,
    ) -> Result<Vec<i64>, Self::Error>;

    /// Add task IDs to a batch
    async fn add_tasks_to_batch(
        &self,
        batch_id: Uuid,
        task_ids: Vec<i64>,
    ) -> Result<(), Self::Error>;

    /// Update the expected task count for a batch (when some queries return no results)
    async fn update_batch_expected_count(
        &self,
        batch_id: Uuid,
        count: i32,
    ) -> Result<(), Self::Error>;

    /// Get a batch by its ID
    async fn get_batch_by_id(
        &self,
        batch_id: Uuid,
    ) -> Result<Option<domain::ProcessingBatch>, Self::Error>;

    /// Ensure workers are allocated in fetcher service
    /// Checks if any workers are active, and if not, allocates the default number
    async fn ensure_workers_allocated(&self) -> Result<(), Self::Error>;

    /// Count how many fetch tasks from a batch have completed
    async fn count_completed_tasks(&self, task_ids: Vec<i64>) -> Result<i32, Self::Error>;

    /// Return the subset of task IDs that are already completed.
    /// Used to build a batch from only already-fetched papers so the user
    /// doesn't have to wait for pending downloads.
    async fn get_completed_task_ids(&self, task_ids: Vec<i64>) -> Result<Vec<i64>, Self::Error>;
}

/// Trait for configuration management
#[async_trait::async_trait]
pub trait ConfigManagement: Send + Sync {
    type Error: Error + Send + Sync;

    /// Get all configuration entries
    async fn get_all_config(&self) -> Result<Vec<ConfigEntry>, Self::Error>;

    /// Update configuration entries
    async fn update_config(
        &self,
        entries: Vec<ConfigEntryUpdate>,
    ) -> Result<Vec<ConfigEntry>, Self::Error>;
}

/// Trait for worker management operations
#[async_trait::async_trait]
pub trait WorkerManagement: Send + Sync {
    type Error: Error + Send + Sync;

    /// Get worker status and statistics
    async fn get_worker_status(&self) -> Result<Vec<WorkerStatus>, Self::Error>;

    /// Allocate workers in the fetcher service
    async fn allocate_workers(
        &self,
        req: AllocateWorkersRequest,
    ) -> Result<WorkerAllocationResponse, Self::Error>;

    /// Stop workers in the fetcher service
    async fn stop_workers(
        &self,
        req: StopWorkersRequest,
    ) -> Result<WorkerStopResponse, Self::Error>;
}

/// Trait for service health checks
#[async_trait::async_trait]
pub trait HealthCheck: Send + Sync {
    type Error: Error + Send + Sync;

    /// Check if the fetcher service is healthy
    async fn fetcher_health(&self) -> Result<(), Self::Error>;

    /// Check if the brainatlas service is healthy
    async fn brainatlas_health(&self) -> Result<(), Self::Error>;
}

/// Trait for the background pipeline runner that continuously discovers and fetches papers
#[async_trait::async_trait]
pub trait PipelineRunner: Send + Sync {
    type Error: Error + Send + Sync;

    /// Phase 1: Generate queries for all regions that don't have any yet.
    /// Returns (regions_processed, queries_generated).
    async fn generate_queries_for_new_regions(&self) -> Result<(usize, usize), Self::Error>;

    /// Phase 2: For ALL regions with queries, re-run ESearch to discover new papers.
    /// UNIQUE(pmc_id) deduplication ensures only genuinely new papers are enqueued.
    /// Returns (regions_scanned, new_tasks_created).
    async fn discover_new_papers(&self) -> Result<(usize, usize), Self::Error>;

    /// Phase 3: Ensure fetcher workers are running. Non-blocking.
    async fn ensure_fetcher_running(&self) -> Result<(), Self::Error>;

    /// Get the count of pending + in_progress fetch tasks.
    async fn get_pending_fetch_task_count(&self) -> Result<i64, Self::Error>;

    /// Get the count of regions that still need query generation (Phase 1 backlog).
    async fn generate_queries_for_new_regions_count(&self) -> Result<i64, Self::Error>;

    /// Get the count of regions that have at least one query (eligible for Phase 2).
    async fn get_regions_with_queries_count(&self) -> Result<i64, Self::Error>;

    /// Get comprehensive system stats for the dev dashboard.
    async fn get_system_stats(&self) -> Result<domain::SystemStats, Self::Error>;
}

pub trait Services:
    CompletionOrchestrator<Error = <Self as Services>::Error>
    + RegionManagement<Error = <Self as Services>::Error>
    + BatchOrchestration<Error = <Self as Services>::Error>
    + ConfigManagement<Error = <Self as Services>::Error>
    + WorkerManagement<Error = <Self as Services>::Error>
    + HealthCheck<Error = <Self as Services>::Error>
    + PipelineRunner<Error = <Self as Services>::Error>
{
    type Error: Error + Send + Sync;
}

impl<E, T> Services for T
where
    T: CompletionOrchestrator<Error = E>
        + RegionManagement<Error = E>
        + BatchOrchestration<Error = E>
        + ConfigManagement<Error = E>
        + WorkerManagement<Error = E>
        + HealthCheck<Error = E>
        + PipelineRunner<Error = E>,
    E: Error + Send + Sync,
{
    type Error = E;
}
