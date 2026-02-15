use crate::batch_orchestration::OrchBatchOrchestration;
use crate::completion_watcher::CompletionWatcher;
use crate::region_management::OrchRegionManagement;
use crate::{Infra, ServiceError};
use app::{BatchOrchestration, CompletionOrchestrator, RegionManagement};
use domain::{
    ConfigKey, PendingTask, PollResult, ProcessResult, ProcessingBatch, RegionQuery, RegionSummary,
};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

pub struct OrchServices<I> {
    completion_watcher: CompletionWatcher<I>,
    region_management: OrchRegionManagement<I>,
    batch_orchestration: OrchBatchOrchestration<I>,
}

impl<I: Infra> OrchServices<I> {
    pub fn new(infra: Arc<I>) -> Self {
        let completion_watcher = CompletionWatcher::new(infra.clone());
        let region_management = OrchRegionManagement::new(infra.clone());
        let batch_orchestration = OrchBatchOrchestration::new(infra);
        Self {
            completion_watcher,
            region_management,
            batch_orchestration,
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

    async fn get_summaries(&self, region_id: i32) -> Result<Vec<RegionSummary>, Self::Error> {
        self.region_management.get_summaries(region_id).await
    }

    async fn get_active_batch(
        &self,
        region_id: i32,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        self.region_management.get_active_batch(region_id).await
    }

    async fn get_queries(&self, region_id: i32) -> Result<Vec<RegionQuery>, Self::Error> {
        self.region_management.get_queries(region_id).await
    }

    async fn store_queries(
        &self,
        region_id: i32,
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
        region_id: i32,
        expected_count: usize,
    ) -> Result<Uuid, Self::Error> {
        self.batch_orchestration
            .create_batch(region_id, expected_count)
            .await
    }

    async fn enqueue_fetch_task(
        &self,
        query: String,
        region_id: i32,
        priority: i32,
    ) -> Result<i64, Self::Error> {
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
}
