use crate::list_brain_regions::BrainAtlasListBrainRegions;
use crate::region_info::BrainAtlasRegionInfo;
use crate::{Infra, ServiceError};
use app::{BrainRegionInfo, ListBrainRegions};
use domain::{BrainRegionEntry, RegionMapping};
use std::sync::Arc;
use uuid::Uuid;

pub struct BrainAtlasServices<I> {
    brain_atlas_list_brain_regions: BrainAtlasListBrainRegions<I>,
    brain_atlas_region_info: BrainAtlasRegionInfo<I>,
}

impl<I: Infra> BrainAtlasServices<I> {
    pub fn new(infra: Arc<I>) -> Self {
        let brain_atlas_list_brain_regions = BrainAtlasListBrainRegions::new(infra.clone());
        let brain_atlas_region_info = BrainAtlasRegionInfo::new(infra);
        Self {
            brain_atlas_list_brain_regions,
            brain_atlas_region_info,
        }
    }
}

#[async_trait::async_trait]
impl<E, I> ListBrainRegions for BrainAtlasServices<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn list(&self) -> Result<Vec<RegionMapping>, Self::Error> {
        self.brain_atlas_list_brain_regions.list().await
    }
}

#[async_trait::async_trait]
impl<E, I> BrainRegionInfo for BrainAtlasServices<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn search(&self, id: Uuid) -> Result<Vec<BrainRegionEntry>, Self::Error> {
        self.brain_atlas_region_info.search(id).await
    }
}
