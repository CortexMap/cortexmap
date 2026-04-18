use domain::{BatchStatus, ConfigKey, ProcessingBatch, RegionQuery};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// Lightweight region info for pipeline queries
#[derive(Debug, Clone)]
pub struct RegionInfo {
    pub id: Uuid,
    pub name: String,
}

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
    async fn get_all_config(&self, database_url: &str) -> Result<Vec<OrchConfig>, Self::Error>;

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

    /// Check if a service is healthy by calling its health endpoint
    async fn check_health(&self, base_url: &str, service_name: &str) -> Result<(), Self::Error>;
}

/// Batch management for region processing
#[async_trait::async_trait]
pub trait BatchManagement: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Get queries for a region
    async fn get_queries(
        &self,
        database_url: &str,
        region_id: Uuid,
    ) -> Result<Vec<RegionQuery>, Self::Error>;

    /// Insert generated queries for a region
    async fn insert_queries(
        &self,
        database_url: &str,
        region_id: Uuid,
        queries: Vec<String>,
    ) -> Result<Vec<Uuid>, Self::Error>;

    /// Delete all queries for a region
    async fn delete_queries(&self, database_url: &str, region_id: Uuid) -> Result<(), Self::Error>;

    /// Create a new processing batch
    async fn create_batch(
        &self,
        database_url: &str,
        region_id: Uuid,
        expected_count: i32,
    ) -> Result<Uuid, Self::Error>;

    /// Add fetch task IDs to a batch
    async fn add_tasks_to_batch(
        &self,
        database_url: &str,
        batch_id: Uuid,
        task_ids: Vec<i64>,
    ) -> Result<(), Self::Error>;

    /// Update the expected task count for a batch
    async fn update_batch_expected_count(
        &self,
        database_url: &str,
        batch_id: Uuid,
        count: i32,
    ) -> Result<(), Self::Error>;

    /// Get a batch by its ID
    async fn get_batch_by_id(
        &self,
        database_url: &str,
        batch_id: Uuid,
    ) -> Result<Option<domain::ProcessingBatch>, Self::Error>;

    /// Get batches by status
    async fn get_batches_by_status(
        &self,
        database_url: &str,
        status: BatchStatus,
    ) -> Result<Vec<ProcessingBatch>, Self::Error>;

    /// Count completed fetch tasks from a list of task IDs
    async fn count_completed_tasks(
        &self,
        database_url: &str,
        task_ids: &[i64],
    ) -> Result<usize, Self::Error>;

    /// Return the subset of `task_ids` whose status is 'completed'.
    /// Used by `generate_summary` to build a batch from only already-fetched papers.
    async fn get_completed_task_ids(
        &self,
        database_url: &str,
        task_ids: &[i64],
    ) -> Result<Vec<i64>, Self::Error>;

    /// Get S3 keys for completed fetch tasks
    async fn get_task_s3_keys(
        &self,
        database_url: &str,
        task_ids: &[i64],
    ) -> Result<Vec<String>, Self::Error>;

    /// Get paper metadata for fetch tasks (s3_key -> pmc_id, uid, query)
    /// JOINs fetch_tasks with fetch_task_components (and papers for uid)
    async fn get_task_paper_metadata(
        &self,
        database_url: &str,
        task_ids: &[i64],
    ) -> Result<Vec<crate::PaperMetadataRecord>, Self::Error>;

    /// Update batch status
    async fn update_batch_status(
        &self,
        database_url: &str,
        batch_id: Uuid,
        status: BatchStatus,
        error: Option<String>,
    ) -> Result<(), Self::Error>;

    /// Mark batch complete with summary
    async fn complete_batch(&self, database_url: &str, batch_id: Uuid) -> Result<(), Self::Error>;

    /// Get active batch for a region
    async fn get_active_batch(
        &self,
        database_url: &str,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error>;

    /// Get most recent batch for a region (regardless of status)
    async fn get_recent_batch(
        &self,
        database_url: &str,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error>;
}

/// Raw system stats from DB queries
#[derive(Debug, Clone, Default)]
pub struct SystemStatsRaw {
    pub fetch_tasks_by_status: Vec<(String, i64)>,
    pub batches_by_status: Vec<(String, i64)>,
    pub total_queries: i64,
    pub regions_with_queries: i64,
    pub query_distribution: Vec<(i64, i64)>,
    pub total_papers: i64,
    pub total_summaries: i64,
}

/// Region mapping information from region_mapping table
#[derive(Debug, Clone)]
pub struct RegionMapping {
    pub id: Uuid,
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub red: Option<i32>,
    pub green: Option<i32>,
    pub blue: Option<i32>,
    pub structure_order: Option<i32>,
    pub parent_region_id: Option<i32>,
    pub parent_acronym: Option<String>,
}

/// Region summary record from database
#[derive(Debug, Clone)]
pub struct RegionSummaryRecord {
    pub id: Uuid,
    pub summary: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub batch_id: Uuid,
}

/// Source chunk metadata from brain_region_embeddings
#[derive(Debug, Clone)]
pub struct ChunkSourceRecord {
    pub id: Uuid,
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_query: Option<String>,
}

/// Paper metadata record derived from fetch_tasks + fetch_task_components
#[derive(Debug, Clone)]
pub struct PaperMetadataRecord {
    pub s3_key: String,
    pub pmc_id: Option<String>,
    pub uid: Option<String>,
    pub query: Option<String>,
}

/// A single search hit from the reverse search query
#[derive(Debug, Clone)]
pub struct SearchHitRecord {
    pub region_uuid: Uuid,
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub summary_snippet: Option<String>,
    pub match_source: String,
    pub rank: f64,
}

#[async_trait::async_trait]
pub trait RegionMappingQueries: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Get region mapping by UUID
    async fn get_region_mapping(
        &self,
        database_url: &str,
        region_uuid: Uuid,
    ) -> Result<Option<RegionMapping>, Self::Error>;

    /// Get all regions from region_mapping table
    async fn get_all_regions(&self, database_url: &str) -> Result<Vec<RegionMapping>, Self::Error>;

    /// Get total count of regions in region_mapping table
    async fn get_total_region_count(&self, database_url: &str) -> Result<i64, Self::Error>;

    /// Count regions that have no batches at all
    async fn count_regions_without_batches(&self, database_url: &str) -> Result<i64, Self::Error>;

    /// Get region summaries by region_id (Int4)
    async fn get_region_summaries(
        &self,
        database_url: &str,
        region_id: i32,
    ) -> Result<Vec<RegionSummaryRecord>, Self::Error>;

    /// Get distinct source chunks for a given summary_id
    async fn get_summary_sources(
        &self,
        database_url: &str,
        summary_id: Uuid,
    ) -> Result<Vec<ChunkSourceRecord>, Self::Error>;

    /// Search regions by natural language query across names, acronyms, and latest summaries.
    /// Returns the limited result set and the total count of matches before limiting.
    async fn search_regions(
        &self,
        database_url: &str,
        query: &str,
        limit: i64,
    ) -> Result<(Vec<SearchHitRecord>, i64), Self::Error>;

    /// Get regions that have zero queries in region_queries.
    /// These need query generation (Phase 1 of pipeline).
    async fn get_regions_without_queries(
        &self,
        database_url: &str,
    ) -> Result<Vec<RegionInfo>, Self::Error>;

    /// Get all regions that have queries (for re-scanning in Phase 2).
    /// Returns (region_id_uuid, region_name, [query_text, ...]) for each region.
    async fn get_all_regions_with_queries(
        &self,
        database_url: &str,
    ) -> Result<Vec<(Uuid, String, Vec<String>)>, Self::Error>;

    /// Count fetch_tasks that are pending or in_progress.
    async fn get_pending_fetch_task_count(
        &self,
        database_url: &str,
    ) -> Result<i64, Self::Error>;

    /// Get comprehensive system stats for the dev dashboard (aggregate counts).
    async fn get_system_stats(
        &self,
        database_url: &str,
    ) -> Result<SystemStatsRaw, Self::Error>;
}

/// Cache client for Redis-backed read-through caching and invalidation.
#[async_trait::async_trait]
pub trait CacheClient: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Fetch a cached value by key. Returns `None` on miss or connection failure.
    async fn cache_get(&self, key: &str) -> Result<Option<String>, Self::Error>;

    /// Store a value with a TTL (seconds). Fire-and-forget semantics — callers
    /// should swallow errors.
    async fn cache_set(&self, key: &str, value: &str, ttl_secs: u64) -> Result<(), Self::Error>;

    /// Delete a single cached key.
    async fn cache_del(&self, key: &str) -> Result<(), Self::Error>;

    /// Delete all keys matching a glob pattern (e.g. `orch:region:*:status`).
    /// Returns the number of keys deleted.
    async fn cache_del_pattern(&self, pattern: &str) -> Result<u64, Self::Error>;
}

/// Blanket: any `T: OrchDatabase + EnvInfra + HttpClient + BatchManagement + RegionMappingQueries + CacheClient` automatically satisfies `Infra`.
pub trait Infra:
    EnvInfra<Error = <Self as Infra>::Error>
    + OrchDatabase<Error = <Self as Infra>::Error>
    + HttpClient<Error = <Self as Infra>::Error>
    + BatchManagement<Error = <Self as Infra>::Error>
    + RegionMappingQueries<Error = <Self as Infra>::Error>
    + CacheClient<Error = <Self as Infra>::Error>
{
    type Error: std::error::Error + Send + Sync + 'static;
}

impl<E, T> Infra for T
where
    T: EnvInfra<Error = E>
        + OrchDatabase<Error = E>
        + HttpClient<Error = E>
        + BatchManagement<Error = E>
        + RegionMappingQueries<Error = E>
        + CacheClient<Error = E>,
    E: std::error::Error + Send + Sync + 'static,
{
    type Error = E;
}
