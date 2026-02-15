use crate::{ApiError, OrchApi};
use app::{AppError, OrchApp, Services};
use domain::{
    ConfigEntry, ConfigEntryUpdate, InvalidateResult, PipelineStatsResult, Priority,
    RegionStatusResult, SearchRegionResult,
};
use std::sync::Arc;
use uuid::Uuid;

pub struct Orch<S> {
    services: Arc<S>,
}

impl<S> Orch<S> {
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }
}

impl<S: Services + 'static> Orch<S> {
    fn app(&self) -> OrchApp<S> {
        OrchApp::new(self.services.clone())
    }
}

#[async_trait::async_trait]
impl<E, S> OrchApi for Orch<S>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    type Error = ApiError<AppError<E>>;

    async fn init(&self) -> Result<(), Self::Error> {
        self.app()
            .init()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn search_region(&self, _region_id: Uuid) -> Result<SearchRegionResult, Self::Error> {
        Err(ApiError::NotImplemented)
    }

    async fn get_region_status(&self, _region_id: Uuid) -> Result<RegionStatusResult, Self::Error> {
        Err(ApiError::NotImplemented)
    }

    async fn invalidate_region(
        &self,
        _region_id: Uuid,
        _priority: Option<Priority>,
    ) -> Result<InvalidateResult, Self::Error> {
        Err(ApiError::NotImplemented)
    }

    async fn get_pipeline_stats(&self) -> Result<PipelineStatsResult, Self::Error> {
        Err(ApiError::NotImplemented)
    }

    async fn get_config(&self) -> Result<Vec<ConfigEntry>, Self::Error> {
        Err(ApiError::NotImplemented)
    }

    async fn update_config(
        &self,
        _entries: Vec<ConfigEntryUpdate>,
    ) -> Result<Vec<ConfigEntry>, Self::Error> {
        Err(ApiError::NotImplemented)
    }
}
