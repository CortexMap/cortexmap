use domain::rpc_types::{BrainRegionListResponse, ProcessRegionResponse, SearchBrainRegionResponse, StatusResponse};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait BrainRegionApi: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Search for all brain region summaries by UUID.
    /// Returns proto response ready to serialise.
    /// POST /api/search
    async fn search_brain_region(&self, id: Option<Uuid>) -> Result<SearchBrainRegionResponse, Self::Error>;

    /// List all brain region mappings.
    /// Returns proto response ready to serialise.
    /// GET /api/list
    async fn list_brain_regions(&self) -> Result<BrainRegionListResponse, Self::Error>;

    /// Get the processing status of a specific brain region by UUID.
    /// POST /api/status
    async fn status(&self, id: Uuid) -> Result<StatusResponse, Self::Error>;

    /// Chunk, embed, and summarize the S3 files for a region, then persist to region_summary.
    /// POST /api/process — called by orch
    async fn process_region(&self, region_id: Option<Uuid>, s3_keys: Vec<String>) -> Result<ProcessRegionResponse, Self::Error>;
}
