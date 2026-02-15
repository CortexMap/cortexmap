use crate::{ApiError, BrainRegionApi};
use app::{AppError, BrainAtlasApp, Services};
use domain::rpc_types::{
    BrainRegionListResponse, ProcessRegionResponse, SearchBrainRegionResponse, StatusResponse,
};
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

    async fn search_brain_region(&self, id: Option<Uuid>) -> Result<SearchBrainRegionResponse, Self::Error> {
        let id = id.ok_or(ApiError::MissingOrInvalidId)?;
        let entries = self.app().search(id).await.map_err(ApiError::AppError)?;
        Ok(SearchBrainRegionResponse {
            entries: entries.into_iter().map(Into::into).collect(),
        })
    }

    async fn list_brain_regions(&self) -> Result<BrainRegionListResponse, Self::Error> {
        let regions = self.app().list().await.map_err(ApiError::AppError)?;
        Ok(BrainRegionListResponse {
            regions: regions.into_iter().map(Into::into).collect(),
        })
    }

    async fn status(&self, _id: Uuid) -> Result<StatusResponse, Self::Error> {
        Err(ApiError::NotImplemented)
    }

    async fn process_region(&self, _region_id: Uuid, _s3_keys: Vec<String>) -> Result<ProcessRegionResponse, Self::Error> {
        Err(ApiError::NotImplemented)
    }
}
