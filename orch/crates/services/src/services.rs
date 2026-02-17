use crate::batch_orchestration::OrchBatchOrchestration;
use crate::completion_watcher::CompletionWatcher;
use crate::config_management::OrchConfigManagement;
use crate::region_management::OrchRegionManagement;
use crate::{Infra, ServiceError};
use app::{
    BatchOrchestration, CompletionOrchestrator, ConfigManagement, HealthCheck, RegionManagement,
};
use domain::{
    ConfigEntry, ConfigEntryUpdate, ConfigKey, PendingTask, PollResult, ProcessResult,
    ProcessingBatch, RegionQuery, RegionSummary,
};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

pub struct OrchServices<I> {
    completion_watcher: CompletionWatcher<I>,
    region_management: OrchRegionManagement<I>,
    batch_orchestration: OrchBatchOrchestration<I>,
    config_management: OrchConfigManagement<I>,
    infra: Arc<I>,
}

impl<I: Infra> OrchServices<I> {
    pub fn new(infra: Arc<I>) -> Self {
        let completion_watcher = CompletionWatcher::new(infra.clone());
        let region_management = OrchRegionManagement::new(infra.clone());
        let batch_orchestration = OrchBatchOrchestration::new(infra.clone());
        let config_management = OrchConfigManagement::new(infra.clone());
        Self {
            completion_watcher,
            region_management,
            batch_orchestration,
            config_management,
            infra,
        }
    }
}

#[async_trait::async_trait]
impl<E, I> CompletionOrchestrator for OrchServices<I>
where
    E: Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn poll(&self) -> Result<PollResult, Self::Error> {
        self.completion_watcher.poll().await
    }

    async fn process(&self, tasks: Vec<PendingTask>) -> Result<ProcessResult, Self::Error> {
        self.completion_watcher.process(tasks).await
    }

    async fn get_config(&self, key: ConfigKey) -> Result<Option<String>, Self::Error> {
        self.completion_watcher.get_config(key).await
    }
}

#[async_trait::async_trait]
impl<E, I> RegionManagement for OrchServices<I>
where
    E: Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn get_summaries(&self, region_id: Uuid) -> Result<Vec<RegionSummary>, Self::Error> {
        self.region_management.get_summaries(region_id).await
    }

    async fn get_active_batch(
        &self,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        self.region_management.get_active_batch(region_id).await
    }

    async fn get_recent_batch(
        &self,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        self.region_management.get_recent_batch(region_id).await
    }

    async fn get_queries(&self, region_id: Uuid) -> Result<Vec<RegionQuery>, Self::Error> {
        self.region_management.get_queries(region_id).await
    }

    async fn store_queries(
        &self,
        region_id: Uuid,
        queries: Vec<String>,
    ) -> Result<Vec<Uuid>, Self::Error> {
        self.region_management
            .store_queries(region_id, queries)
            .await
    }

    async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, Self::Error> {
        self.region_management
            .generate_queries(region_name, count)
            .await
    }

    async fn update_batch_status(
        &self,
        batch_id: Uuid,
        status: domain::BatchStatus,
        error: Option<String>,
    ) -> Result<(), Self::Error> {
        self.region_management
            .update_batch_status(batch_id, status, error)
            .await
    }

    async fn get_batches_by_status(
        &self,
        status: domain::BatchStatus,
    ) -> Result<Vec<domain::ProcessingBatch>, Self::Error> {
        self.region_management.get_batches_by_status(status).await
    }

    async fn get_region_name(&self, region_id: Uuid) -> Result<String, Self::Error> {
        self.region_management.get_region_name(region_id).await
    }

    async fn get_total_regions(&self) -> Result<i64, Self::Error> {
        self.region_management.get_total_regions().await
    }

    async fn count_regions_without_batches(&self) -> Result<i64, Self::Error> {
        self.region_management.count_regions_without_batches().await
    }

    async fn get_query_generation_limit(&self) -> Result<Option<u32>, Self::Error> {
        self.region_management.get_query_generation_limit().await
    }

    async fn get_all_regions(&self) -> Result<Vec<domain::Region>, Self::Error> {
        self.region_management.get_all_regions().await
    }
    
    async fn delete_queries(&self, region_id: Uuid) -> Result<(), Self::Error> {
        self.region_management.delete_queries(region_id).await
    }

    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<domain::ChunkSourceResponse, Self::Error> {
        self.region_management.get_chunk_source(chunk_id).await
    }
}

#[async_trait::async_trait]
impl<E, I> ConfigManagement for OrchServices<I>
where
    E: Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn get_all_config(&self) -> Result<Vec<ConfigEntry>, Self::Error> {
        self.config_management.get_all_config().await
    }

    async fn update_config(
        &self,
        entries: Vec<ConfigEntryUpdate>,
    ) -> Result<Vec<ConfigEntry>, Self::Error> {
        self.config_management.update_config(entries).await
    }
}

#[async_trait::async_trait]
impl<E, I> BatchOrchestration for OrchServices<I>
where
    E: Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn create_batch(
        &self,
        region_id: Uuid,
        expected_count: usize,
    ) -> Result<Uuid, Self::Error> {
        self.batch_orchestration
            .create_batch(region_id, expected_count)
            .await
    }

    async fn enqueue_fetch_task(
        &self,
        query: String,
        region_id: Uuid,
        priority: i32,
    ) -> Result<Vec<i64>, Self::Error> {
        self.batch_orchestration
            .enqueue_fetch_task(query, region_id, priority)
            .await
    }

    async fn add_tasks_to_batch(
        &self,
        batch_id: Uuid,
        task_ids: Vec<i64>,
    ) -> Result<(), Self::Error> {
        self.batch_orchestration
            .add_tasks_to_batch(batch_id, task_ids)
            .await
    }

    async fn update_batch_expected_count(&self, batch_id: Uuid, count: i32) -> Result<(), Self::Error> {
        self.batch_orchestration
            .update_batch_expected_count(batch_id, count)
            .await
    }

    async fn get_batch_by_id(&self, batch_id: Uuid) -> Result<Option<domain::ProcessingBatch>, Self::Error> {
        self.batch_orchestration
            .get_batch_by_id(batch_id)
            .await
    }

    async fn ensure_workers_allocated(&self) -> Result<(), Self::Error> {
        self.batch_orchestration
            .ensure_workers_allocated()
            .await
    }
}

#[async_trait::async_trait]
impl<E, I> HealthCheck for OrchServices<I>
where
    E: Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn fetcher_health(&self) -> Result<(), Self::Error> {
        // Normalize URL helper
        fn normalize_url(addr: &str) -> String {
            if addr.starts_with("http://") || addr.starts_with("https://") {
                addr.to_string()
            } else {
                let replaced = addr.replace("0.0.0.0", "localhost");
                format!("http://{}", replaced)
            }
        }

        let fetcher_addr = self.infra.get_env_var("FETCHER_HTTP_ADDR").map_err(|e| {
            ServiceError::ConfigNotFound {
                key: format!("FETCHER_HTTP_ADDR environment variable: {}", e),
            }
        })?;

        let fetcher_url = format!("{}/fetcher-be", normalize_url(&fetcher_addr));

        self.infra
            .check_health(&fetcher_url, "fetcher")
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn brainatlas_health(&self) -> Result<(), Self::Error> {
        // Normalize URL helper
        fn normalize_url(addr: &str) -> String {
            if addr.starts_with("http://") || addr.starts_with("https://") {
                addr.to_string()
            } else {
                let replaced = addr.replace("0.0.0.0", "localhost");
                format!("http://{}", replaced)
            }
        }

        let brainatlas_addr = self
            .infra
            .get_env_var("BRAINATLAS_HTTP_ADDR")
            .map_err(|e| ServiceError::ConfigNotFound {
                key: format!("BRAINATLAS_HTTP_ADDR environment variable: {}", e),
            })?;

        let brainatlas_url = format!("{}/brainatlas-be", normalize_url(&brainatlas_addr));

        self.infra
            .check_health(&brainatlas_url, "brainatlas")
            .await
            .map_err(ServiceError::InfraError)
    }
}
