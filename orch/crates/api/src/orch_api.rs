use crate::{ApiError, OrchApi};
use app::{AppError, OrchApp, Services};
use domain::{
    BatchStatusResult, ConfigEntry, ConfigEntryUpdate, GenerateSummaryResult, PipelineStatsResult,
    Region, RegionStatusResult, SearchRegionResult,
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

    async fn list_summaries(&self, region_id: Uuid) -> Result<SearchRegionResult, Self::Error> {
        self.app()
            .list_summaries(region_id)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn generate_summary(&self, region_id: Uuid) -> Result<GenerateSummaryResult, Self::Error> {
        self.app()
            .generate_summary(region_id)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn get_batch_status(&self, batch_id: Uuid) -> Result<BatchStatusResult, Self::Error> {
        self.app()
            .get_batch_status(batch_id)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn get_region_status(&self, region_id: Uuid) -> Result<RegionStatusResult, Self::Error> {
        self.app()
            .get_region_status(region_id)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn get_pipeline_stats(&self) -> Result<PipelineStatsResult, Self::Error> {
        self.app()
            .get_pipeline_stats()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn get_config(&self) -> Result<Vec<ConfigEntry>, Self::Error> {
        self.app()
            .get_config()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn update_config(
        &self,
        entries: Vec<ConfigEntryUpdate>,
    ) -> Result<Vec<ConfigEntry>, Self::Error> {
        self.app()
            .update_config(entries)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn get_all_regions(&self) -> Result<Vec<Region>, Self::Error> {
        self.app()
            .get_all_regions()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }
    
    async fn fetcher_health(&self) -> Result<(), Self::Error> {
        self.services
            .fetcher_health()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }
    
    async fn brainatlas_health(&self) -> Result<(), Self::Error> {
        self.services
            .brainatlas_health()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }
}
