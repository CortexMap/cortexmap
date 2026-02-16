use crate::{BatchManagement, EnvInfra, HttpClient, ServiceError};
use app::BatchOrchestration;
use domain::ConfigKey;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

pub struct OrchBatchOrchestration<I> {
    infra: Arc<I>,
}

impl<I> OrchBatchOrchestration<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }

    /// Normalize HTTP address to full URL
    /// Converts "0.0.0.0:8080" to "http://localhost:8080"
    fn normalize_url(addr: &str) -> String {
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.to_string()
        } else {
            let host_port = addr.replace("0.0.0.0", "localhost");
            format!("http://{}", host_port)
        }
    }
}

#[derive(Debug, Serialize)]
struct EnqueueRequest {
    query: String,
    page_size: u32,
    max_retry_attempts: u32,
}

#[derive(Debug, Deserialize)]
struct EnqueueResponse {
    success: bool,
    tasks_enqueued: u32,
    pmc_ids: Vec<String>,
    task_ids: Vec<i64>,
    error_message: String,
}

#[derive(Debug, Serialize)]
struct AllocateWorkersRequest {
    worker_count: u32,
    task_timeout_secs: u64,
    max_retry_attempts: u32,
}

#[derive(Debug, Deserialize)]
struct AllocateWorkersResponse {
    success: bool,
    worker_ids: Vec<String>,
    error_message: String,
}

#[derive(Debug, Deserialize)]
struct WorkerStatusResponse {
    workers: Vec<WorkerInfo>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WorkerInfo {
    worker_id: String,
    status: String,
}


#[async_trait::async_trait]
impl<E, I> BatchOrchestration for OrchBatchOrchestration<I>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E> + HttpClient<Error = E> + BatchManagement<Error = E> + crate::OrchDatabase<Error = E> + Send + Sync,
{
    type Error = ServiceError<E>;

    async fn create_batch(&self, region_id: Uuid, expected_count: usize) -> Result<Uuid, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .create_batch(&database_url, region_id, expected_count as i32)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn enqueue_fetch_task(
        &self,
        query: String,
        region_id: Uuid,
        _priority: i32,
    ) -> Result<Vec<i64>, Self::Error> {
        // Try env var first, fall back to config
        let fetcher_url = match self.infra.get_env_var("FETCHER_HTTP_ADDR") {
            Ok(addr) => Self::normalize_url(&addr),
            Err(_) => {
                let database_url = self
                    .infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;
                
                self.infra
                    .get_config(&database_url, ConfigKey::FetcherBaseUrl)
                    .await
                    .map_err(ServiceError::InfraError)?
                    .ok_or_else(|| ServiceError::ConfigNotFound {
                        key: "fetcher_base_url".to_string(),
                    })?
            }
        };

        let url = format!("{}/fetcher-be/api/queue/enqueue", fetcher_url.trim_end_matches('/'));

        let request = EnqueueRequest {
            query: query.clone(),
            page_size: 20, // Get up to 20 papers per query
            max_retry_attempts: 3,
        };

        tracing::info!(url = %url, region_id = %region_id, query = %request.query, "Calling fetcher enqueue");

        let response: EnqueueResponse = self
            .infra
            .post(&url, &request)
            .await
            .map_err(ServiceError::InfraError)?;

        if !response.success {
            return Err(ServiceError::External {
                message: format!("Fetcher enqueue failed: {}", response.error_message),
            });
        }

        tracing::info!(
            tasks_enqueued = response.tasks_enqueued,
            pmc_count = response.pmc_ids.len(),
            task_ids_count = response.task_ids.len(),
            region_id = %region_id,
            "Successfully enqueued fetch tasks"
        );

        // Return all task IDs created by this query
        Ok(response.task_ids)
    }

    async fn add_tasks_to_batch(&self, batch_id: Uuid, task_ids: Vec<i64>) -> Result<(), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .add_tasks_to_batch(&database_url, batch_id, task_ids)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn update_batch_expected_count(&self, batch_id: Uuid, count: i32) -> Result<(), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .update_batch_expected_count(&database_url, batch_id, count)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn get_batch_by_id(&self, batch_id: Uuid) -> Result<Option<domain::ProcessingBatch>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_batch_by_id(&database_url, batch_id)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn ensure_workers_allocated(&self) -> Result<(), Self::Error> {
        // Get fetcher URL
        let fetcher_url = match self.infra.get_env_var("FETCHER_HTTP_ADDR") {
            Ok(addr) => Self::normalize_url(&addr),
            Err(_) => {
                let database_url = self
                    .infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;
                
                self.infra
                    .get_config(&database_url, ConfigKey::FetcherBaseUrl)
                    .await
                    .map_err(ServiceError::InfraError)?
                    .ok_or_else(|| ServiceError::ConfigNotFound {
                        key: "fetcher_base_url".to_string(),
                    })?
            }
        };

        // Check current worker status
        let worker_status_url = format!("{}/fetcher-be/api/queue/workers/status", fetcher_url.trim_end_matches('/'));
        
        tracing::debug!(url = %worker_status_url, "Checking worker status");
        
        let worker_status: WorkerStatusResponse = self
            .infra
            .get(&worker_status_url)
            .await
            .map_err(ServiceError::InfraError)?;

        // Count active workers
        let active_workers = worker_status.workers.iter()
            .filter(|w| w.status == "running")
            .count();

        tracing::info!(active_workers = active_workers, "Current worker count");

        // If no workers are active, allocate default number
        if active_workers == 0 {
            let database_url = self
                .infra
                .get_env_var("DATABASE_URL")
                .map_err(ServiceError::InfraError)?;
            
            let default_worker_count: u32 = self.infra
                .get_config(&database_url, ConfigKey::DefaultWorkerCount)
                .await
                .map_err(ServiceError::InfraError)?
                .and_then(|s| s.parse().ok())
                .unwrap_or(2); // Fallback to 2 if not configured

            tracing::info!(worker_count = default_worker_count, "No workers active, allocating default workers");

            let allocate_url = format!("{}/fetcher-be/api/queue/workers/allocate", fetcher_url.trim_end_matches('/'));
            
            let request = AllocateWorkersRequest {
                worker_count: default_worker_count,
                task_timeout_secs: 300,
                max_retry_attempts: 3,
            };

            let response: AllocateWorkersResponse = self
                .infra
                .post(&allocate_url, &request)
                .await
                .map_err(ServiceError::InfraError)?;

            if !response.success {
                return Err(ServiceError::External {
                    message: format!("Failed to allocate workers: {}", response.error_message),
                });
            }

            tracing::info!(
                worker_ids = ?response.worker_ids,
                count = response.worker_ids.len(),
                "Successfully allocated workers"
            );
        } else {
            tracing::info!(active_workers = active_workers, "Workers already active, skipping allocation");
        }

        Ok(())
    }
}
