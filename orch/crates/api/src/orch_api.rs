use crate::{ApiError, OrchApi};
use app::{AppError, OrchApp, Services};
use domain::{
    AllocateWorkersRequest, BatchStatusResult, ChunkSourceResponse, ConfigEntry, ConfigEntryUpdate,
    GenerateSummaryResult, PipelineHealthStatus, PipelineStatsResult, PipelineTriggerRequest,
    PipelineTriggerResult, RedisStats, Region, RegionStatusResult, SearchRegionResult,
    SearchResponse, StopWorkersRequest, SystemStats, WorkerAllocationResponse, WorkerStatus,
    WorkerStopResponse,
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

    async fn generate_summary(
        &self,
        region_id: Uuid,
    ) -> Result<GenerateSummaryResult, Self::Error> {
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

    async fn get_active_batch(&self, region_id: Uuid) -> Result<Option<Uuid>, Self::Error> {
        self.app()
            .get_active_batch_id(region_id)
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

    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<ChunkSourceResponse, Self::Error> {
        self.services
            .get_chunk_source(chunk_id)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn get_worker_status(&self) -> Result<Vec<WorkerStatus>, Self::Error> {
        self.services
            .get_worker_status()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn allocate_workers(
        &self,
        req: AllocateWorkersRequest,
    ) -> Result<WorkerAllocationResponse, Self::Error> {
        self.services
            .allocate_workers(req)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn stop_workers(
        &self,
        req: StopWorkersRequest,
    ) -> Result<WorkerStopResponse, Self::Error> {
        self.services
            .stop_workers(req)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn reverse_search(&self, query: String) -> Result<SearchResponse, Self::Error> {
        self.app()
            .reverse_search(&query)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn get_pipeline_status(&self) -> Result<PipelineHealthStatus, Self::Error> {
        self.app()
            .get_pipeline_status()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn get_system_stats(&self) -> Result<SystemStats, Self::Error> {
        self.app()
            .get_system_stats()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn trigger_pipeline(
        &self,
        req: PipelineTriggerRequest,
    ) -> Result<PipelineTriggerResult, Self::Error> {
        self.app()
            .trigger_pipeline(req)
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }

    async fn get_redis_stats(&self) -> Result<RedisStats, Self::Error> {
        self.app()
            .get_redis_stats()
            .await
            .map_err(|e| ApiError::AppError(AppError::ServiceError(e)))
    }
}
