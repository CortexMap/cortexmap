use domain::{BrainRegionEntry, RegionMapping, Status};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait BrainRegionApi: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Search for a specific brain region entry by UUID
    /// POST /api/search
    async fn search_brain_region(&self, id: Uuid) -> Result<BrainRegionEntry, Self::Error>;

    /// List all brain region mappings
    /// GET /api/list
    async fn list_brain_regions(&self) -> Result<Vec<RegionMapping>, Self::Error>;

    /// Get the processing status of a specific brain region by UUID
    /// POST /api/status
    async fn status(&self, id: Uuid) -> Result<Status, Self::Error>;
}
