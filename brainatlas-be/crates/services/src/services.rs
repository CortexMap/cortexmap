use crate::list_brain_regions::BrainAtlasListBrainRegions;
use crate::{Infra, ServiceError};
use app::ListBrainRegions;
use domain::RegionMapping;
use std::sync::Arc;

pub struct BrainAtlasServices<I> {
    brain_atlas_list_brain_regions: BrainAtlasListBrainRegions<I>,
}

impl<I: Infra> BrainAtlasServices<I> {
    pub fn new(infra: Arc<I>) -> Self {
        let brain_atlas_list_brain_regions = BrainAtlasListBrainRegions::new(infra);
        Self {
            brain_atlas_list_brain_regions,
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
