use domain::{
    ConfigEntry, ConfigEntryUpdate, InvalidateResult, PipelineStatsResult, Priority,
    RegionStatusResult, SearchRegionResult,
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
    
    /// Search for region summaries by region UUID
    /// If no summary exists, triggers the pipeline at USER_REQUESTED priority
    /// Returns current status and any existing summaries
    async fn search_region(&self, region_id: Uuid) -> Result<SearchRegionResult, Self::Error>;
    
    /// Get the end-to-end pipeline status for a single region
    async fn get_region_status(&self, region_id: Uuid) -> Result<RegionStatusResult, Self::Error>;
    
    /// Queue a fresh fetch + process cycle for a region
    /// Existing summaries remain readable while new cycle runs
    async fn invalidate_region(
        &self,
        region_id: Uuid,
        priority: Option<Priority>,
    ) -> Result<InvalidateResult, Self::Error>;
    
    /// Get high-level count breakdown across all regions
    async fn get_pipeline_stats(&self) -> Result<PipelineStatsResult, Self::Error>;
    
    /// Read current orch configuration
    async fn get_config(&self) -> Result<Vec<ConfigEntry>, Self::Error>;
    
    /// Update one or more config entries at runtime without restart
    async fn update_config(&self, entries: Vec<ConfigEntryUpdate>) -> Result<Vec<ConfigEntry>, Self::Error>;
}
