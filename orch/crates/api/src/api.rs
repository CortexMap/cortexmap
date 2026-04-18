use domain::{
    AllocateWorkersRequest, BatchStatusResult, ChunkSourceResponse, ConfigEntry, ConfigEntryUpdate,
    GenerateSummaryResult, PipelineHealthStatus, PipelineStatsResult, PipelineTriggerRequest,
    PipelineTriggerResult, RedisStats, Region, RegionStatusResult, SearchRegionResult,
    SearchResponse, StopWorkersRequest, SystemStats, WorkerAllocationResponse, WorkerStatus,
    WorkerStopResponse,
};
use uuid::Uuid;

/// Main API trait for the Orch service
/// All methods correspond to RPC endpoints defined in orch.proto
#[async_trait::async_trait]
pub trait OrchApi: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Initialize the orchestrator
    /// Spawns background tasks (completion watcher loop)
    async fn init(&self) -> Result<(), Self::Error>;

    /// List all summaries for a region (just returns summaries with metadata)
    async fn list_summaries(&self, region_id: Uuid) -> Result<SearchRegionResult, Self::Error>;

    /// Generate a new summary for a region
    /// Creates a new batch, generates queries, enqueues tasks
    /// Returns batch_id immediately for tracking progress
    async fn generate_summary(&self, region_id: Uuid)
    -> Result<GenerateSummaryResult, Self::Error>;

    /// Get the status of a specific batch
    async fn get_batch_status(&self, batch_id: Uuid) -> Result<BatchStatusResult, Self::Error>;

    /// Get the active batch for a region (if one is in progress)
    /// Returns the batch ID if there's an active batch (collecting, ready, or processing)
    /// Returns None if no active batch exists
    async fn get_active_batch(&self, region_id: Uuid) -> Result<Option<Uuid>, Self::Error>;

    /// Get the end-to-end pipeline status for a single region
    async fn get_region_status(&self, region_id: Uuid) -> Result<RegionStatusResult, Self::Error>;

    /// Get high-level count breakdown across all regions
    async fn get_pipeline_stats(&self) -> Result<PipelineStatsResult, Self::Error>;

    /// Read current orch configuration
    async fn get_config(&self) -> Result<Vec<ConfigEntry>, Self::Error>;

    /// Update one or more config entries at runtime without restart
    async fn update_config(
        &self,
        entries: Vec<ConfigEntryUpdate>,
    ) -> Result<Vec<ConfigEntry>, Self::Error>;

    /// Get all brain regions from region_mapping table
    async fn get_all_regions(&self) -> Result<Vec<Region>, Self::Error>;

    /// Health check for fetcher service
    async fn fetcher_health(&self) -> Result<(), Self::Error>;

    /// Health check for brainatlas service
    async fn brainatlas_health(&self) -> Result<(), Self::Error>;

    /// Resolve a chunk UUID to its full source details
    /// Forwards to brainatlas-be: GET /brainatlas-be/api/chunks/{chunk_id}/source
    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<ChunkSourceResponse, Self::Error>;

    /// Get worker status and statistics
    /// Forwards to fetcher-be: GET /fetcher-be/api/queue/workers/status
    async fn get_worker_status(&self) -> Result<Vec<WorkerStatus>, Self::Error>;

    /// Allocate workers in the fetcher service
    /// Forwards to fetcher-be: POST /fetcher-be/api/queue/workers/allocate
    async fn allocate_workers(
        &self,
        req: AllocateWorkersRequest,
    ) -> Result<WorkerAllocationResponse, Self::Error>;

    /// Stop workers in the fetcher service
    /// Forwards to fetcher-be: POST /fetcher-be/api/queue/workers/stop
    async fn stop_workers(
        &self,
        req: StopWorkersRequest,
    ) -> Result<WorkerStopResponse, Self::Error>;

    /// Reverse search: find brain regions by natural language query
    /// Searches across region names, acronyms, and latest summaries
    async fn reverse_search(&self, query: String) -> Result<SearchResponse, Self::Error>;

    /// Lightweight pipeline health snapshot: region/query/task counts + active workers
    async fn get_pipeline_status(&self) -> Result<PipelineHealthStatus, Self::Error>;

    /// Get comprehensive system statistics for the dev dashboard
    async fn get_system_stats(&self) -> Result<SystemStats, Self::Error>;

    /// Manually trigger pipeline phases on demand. Each phase is opt-in via
    /// the request body so clients can e.g. only rediscover papers without
    /// regenerating queries. Phases run sequentially in the fixed order:
    /// reset -> generate_queries -> discover_papers -> ensure_workers.
    async fn trigger_pipeline(
        &self,
        req: PipelineTriggerRequest,
    ) -> Result<PipelineTriggerResult, Self::Error>;

    /// Snapshot of the Redis cache used by orch (connection state, key counts
    /// per prefix, memory usage, hit rate). Always succeeds: a Redis outage
    /// surfaces as `connected: false` with an `error` string.
    async fn get_redis_stats(&self) -> Result<RedisStats, Self::Error>;
}
