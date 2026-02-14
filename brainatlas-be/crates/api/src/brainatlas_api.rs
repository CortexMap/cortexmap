use crate::{ApiError, BrainRegionApi};
use app::{AppError, BrainAtlasApp, Services};
use domain::{BrainRegionEntry, RegionMapping, Status};
use std::sync::Arc;
use uuid::Uuid;

pub struct BrainAtlasApi<S> {
    services: Arc<S>,
}

impl<S> BrainAtlasApi<S> {
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }
}

impl<S: Services + 'static> BrainAtlasApi<S> {
    fn app(&self) -> BrainAtlasApp<S> {
        BrainAtlasApp::new(self.services.clone())
    }
}

#[async_trait::async_trait]
impl<E, S> BrainRegionApi for BrainAtlasApi<S>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    type Error = ApiError<AppError<E>>;

    async fn search_brain_region(&self, _id: Uuid) -> Result<BrainRegionEntry, Self::Error> {
        Err(ApiError::NotImplemented)
    }

    async fn list_brain_regions(&self) -> Result<Vec<RegionMapping>, Self::Error> {
        self.app().list().await.map_err(ApiError::AppError)
    }

    async fn status(&self, _id: Uuid) -> Result<Status, Self::Error> {
        Err(ApiError::NotImplemented)
    }
}
