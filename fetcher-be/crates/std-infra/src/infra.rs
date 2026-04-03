use crate::StdDatabaseInfra;
use crate::database::DbPool;
use crate::env::FetcherEnvInfra;
use crate::http::StdHttpInfra;
use crate::s3::StdS3Infra;
use crate::task_queue::StdTaskQueue;
use bytes::Bytes;
use cortexmap_infra::{
    ComponentType, ContentType, DatabaseInfra, EnvInfra, FetchTask, FetchTaskComponent, HttpInfra,
    InfraError, NewFetchTaskLog, NewPaper, Paper, S3Infra, TaskQueueInfra, TaskStats, TaskStatus,
};
use futures::Stream;
use reqwest::Response;
use std::pin::Pin;

pub struct StdInfra {
    env_infra: FetcherEnvInfra,
    http_infra: StdHttpInfra,
    db_infra: StdDatabaseInfra,
    s3_infra: StdS3Infra,
    task_queue: StdTaskQueue,
}

impl StdInfra {
    #[allow(clippy::result_large_err)]
    pub fn new(
        database_url: &str,
        endpoint: &str,
        access_key: &str,
        secret_key: &str,
        bucket: &str,
    ) -> Result<Self, InfraError> {
        let env_infra = FetcherEnvInfra::new();
        let http_infra = StdHttpInfra::new();
        let db_infra = StdDatabaseInfra::new(database_url)?;
        let s3_infra = StdS3Infra::new(endpoint, access_key, secret_key, bucket);

        // Reuse the database pool for task queue
        let task_queue = StdTaskQueue::new(db_infra.pool.clone());

        Ok(Self {
            env_infra,
            http_infra,
            db_infra,
            s3_infra,
            task_queue,
        })
    }

    /// Get the database connection pool for direct queries
    pub fn db_pool(&self) -> &DbPool {
        &self.db_infra.pool
    }
}

impl EnvInfra for StdInfra {
    fn get_env_var(&self, key: &str) -> Result<String, InfraError> {
        self.env_infra.get_env_var(key)
    }
}

#[async_trait::async_trait]
impl HttpInfra for StdInfra {
    async fn get(&self, url: &str) -> Result<Response, InfraError> {
        self.http_infra.get(url).await
    }

    async fn post(&self, url: &str, body: Option<Bytes>) -> Result<Response, InfraError> {
        self.http_infra.post(url, body).await
    }
}

#[async_trait::async_trait]
impl DatabaseInfra for StdInfra {
    async fn insert_paper(&self, new_paper: NewPaper) -> Result<Paper, InfraError> {
        self.db_infra.insert_paper(new_paper).await
    }
}

#[async_trait::async_trait]
impl S3Infra for StdInfra {
    async fn put_s3(
        &self,
        key: &str,
        content_type: ContentType,
        content: Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>,
    ) -> Result<(), InfraError> {
        self.s3_infra.put_s3(key, content_type, content).await
    }

    async fn get_s3(&self, key: &str) -> Result<String, InfraError> {
        self.s3_infra.get_s3(key).await
    }
}

#[async_trait::async_trait]
impl TaskQueueInfra for StdInfra {
    async fn enqueue_task(
        &self,
        pmc_id: String,
        query: String,
        max_attempts: i32,
    ) -> Result<FetchTask, InfraError> {
        self.task_queue
            .enqueue_task(pmc_id, query, max_attempts)
            .await
    }

    async fn get_next_pending_task(
        &self,
        timeout_secs: u64,
    ) -> Result<Option<FetchTask>, InfraError> {
        self.task_queue.get_next_pending_task(timeout_secs).await
    }

    async fn mark_task_started(&self, task_id: i64) -> Result<(), InfraError> {
        self.task_queue.mark_task_started(task_id).await
    }

    async fn mark_task_completed(&self, task_id: i64) -> Result<(), InfraError> {
        self.task_queue.mark_task_completed(task_id).await
    }

    async fn mark_task_failed(&self, task_id: i64, error: String) -> Result<(), InfraError> {
        self.task_queue.mark_task_failed(task_id, error).await
    }

    async fn get_pending_components(
        &self,
        task_id: i64,
    ) -> Result<Vec<FetchTaskComponent>, InfraError> {
        self.task_queue.get_pending_components(task_id).await
    }

    async fn update_component_status(
        &self,
        task_id: i64,
        component_type: ComponentType,
        status: TaskStatus,
        s3_key: Option<String>,
        error: Option<String>,
    ) -> Result<(), InfraError> {
        self.task_queue
            .update_component_status(task_id, component_type, status, s3_key, error)
            .await
    }

    async fn increment_component_attempt(
        &self,
        task_id: i64,
        component_type: ComponentType,
    ) -> Result<i32, InfraError> {
        self.task_queue
            .increment_component_attempt(task_id, component_type)
            .await
    }

    async fn all_components_completed(&self, task_id: i64) -> Result<bool, InfraError> {
        self.task_queue.all_components_completed(task_id).await
    }

    async fn reset_stale_tasks(&self, timeout_secs: u64) -> Result<usize, InfraError> {
        self.task_queue.reset_stale_tasks(timeout_secs).await
    }

    async fn log_task_event(&self, log: NewFetchTaskLog) -> Result<(), InfraError> {
        self.task_queue.log_task_event(log).await
    }

    async fn get_task_stats(&self) -> Result<TaskStats, InfraError> {
        self.task_queue.get_task_stats().await
    }

    async fn get_detailed_task_stats(
        &self,
    ) -> Result<cortexmap_infra::DetailedTaskStats, InfraError> {
        self.task_queue.get_detailed_task_stats().await
    }

    async fn get_component_stats(&self) -> Result<cortexmap_infra::ComponentStats, InfraError> {
        self.task_queue.get_component_stats().await
    }

    async fn get_recent_tasks(
        &self,
        limit: i64,
    ) -> Result<Vec<cortexmap_infra::RecentTaskInfo>, InfraError> {
        self.task_queue.get_recent_tasks(limit).await
    }

    async fn get_task_by_pmc_id(&self, pmc_id: &str) -> Result<Option<FetchTask>, InfraError> {
        self.task_queue.get_task_by_pmc_id(pmc_id).await
    }

    async fn get_task_by_id(&self, task_id: i64) -> Result<Option<FetchTask>, InfraError> {
        self.task_queue.get_task_by_id(task_id).await
    }

    async fn get_tasks_by_status(
        &self,
        status: &str,
        limit: i32,
    ) -> Result<Vec<FetchTask>, InfraError> {
        self.task_queue.get_tasks_by_status(status, limit).await
    }

    async fn get_task_components(
        &self,
        task_id: i64,
    ) -> Result<Vec<FetchTaskComponent>, InfraError> {
        self.task_queue.get_task_components(task_id).await
    }

    // Worker heartbeat management
    async fn claim_task_for_worker(
        &self,
        task_id: i64,
        worker_id: String,
        worker_version: Option<String>,
    ) -> Result<(), InfraError> {
        self.task_queue
            .claim_task_for_worker(task_id, worker_id, worker_version)
            .await
    }

    async fn update_task_heartbeat(&self, task_id: i64) -> Result<(), InfraError> {
        self.task_queue.update_task_heartbeat(task_id).await
    }

    async fn release_worker_tasks(&self, worker_id: String) -> Result<usize, InfraError> {
        self.task_queue.release_worker_tasks(worker_id).await
    }

    async fn release_task(&self, task_id: i64) -> Result<(), InfraError> {
        self.task_queue.release_task(task_id).await
    }

    async fn release_stale_tasks_by_heartbeat(
        &self,
        timeout_secs: u64,
    ) -> Result<usize, InfraError> {
        self.task_queue
            .release_stale_tasks_by_heartbeat(timeout_secs)
            .await
    }
}
