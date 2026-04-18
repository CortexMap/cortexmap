use crate::cache_keys::{self, cached_or_fetch, invalidate, invalidate_pattern};
use crate::{BatchManagement, CacheClient, EnvInfra, HttpClient, ServiceError};
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

impl<E, I> OrchBatchOrchestration<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: EnvInfra<Error = E> + crate::OrchDatabase<Error = E> + Send + Sync,
{
    /// Build a `FetcherRetryConfig` by reading all retry-related keys from orch_config.
    /// Missing or unparseable values fall back to sensible defaults.
    async fn build_retry_config_from_db(
        &self,
        database_url: &str,
    ) -> Result<domain::FetcherRetryConfig, ServiceError<E>> {
        let backoff_strategy = self
            .infra
            .get_config(database_url, ConfigKey::FetcherBackoffStrategy)
            .await
            .map_err(ServiceError::InfraError)?
            .unwrap_or_else(|| "constant".to_string());

        let max_delay_secs: Option<u64> = self
            .infra
            .get_config(database_url, ConfigKey::FetcherMaxDelaySecs)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|s| s.parse().ok());

        let jitter: Option<f64> = self
            .infra
            .get_config(database_url, ConfigKey::FetcherBackoffJitter)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|s| s.parse().ok())
            .filter(|&v: &f64| v > 0.0);

        let empty_queue_sleep_secs: Option<u64> = self
            .infra
            .get_config(database_url, ConfigKey::FetcherEmptyQueueSleepSecs)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|s| s.parse().ok());

        let stale_task_multiplier: Option<u64> = self
            .infra
            .get_config(database_url, ConfigKey::FetcherStaleTaskMultiplier)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|s| s.parse().ok());

        let summary_max_retries: Option<u32> = self
            .infra
            .get_config(database_url, ConfigKey::FetcherSummaryMaxRetries)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|s| s.parse().ok());

        let abstract_max_retries: Option<u32> = self
            .infra
            .get_config(database_url, ConfigKey::FetcherAbstractMaxRetries)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|s| s.parse().ok());

        let pdf_max_retries: Option<u32> = self
            .infra
            .get_config(database_url, ConfigKey::FetcherPdfMaxRetries)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|s| s.parse().ok());

        Ok(domain::FetcherRetryConfig {
            backoff_strategy,
            max_delay_secs,
            jitter,
            empty_queue_sleep_secs,
            stale_task_multiplier,
            summary_max_retries,
            abstract_max_retries,
            pdf_max_retries,
            device_cooldown_secs: None, // Reserved for device-subscription v2
        })
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
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_config: Option<domain::FetcherRetryConfig>,
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
struct WorkerInfo {
    worker_id: String,
    status: String,
    #[serde(default)]
    current_task: Option<String>,
    #[serde(default)]
    tasks_processed: i64,
    #[serde(default)]
    started_at: i64,
    #[serde(default)]
    worker_version: Option<String>,
    #[serde(default)]
    last_heartbeat_at: Option<i64>,
    #[serde(default)]
    uptime_seconds: f64,
    #[serde(default)]
    tasks_failed: i64,
    #[serde(default)]
    success_rate: f64,
}

#[async_trait::async_trait]
impl<E, I> BatchOrchestration for OrchBatchOrchestration<I>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E>
        + HttpClient<Error = E>
        + BatchManagement<Error = E>
        + crate::OrchDatabase<Error = E>
        + CacheClient<Error = E>
        + Send
        + Sync,
{
    type Error = ServiceError<E>;

    async fn create_batch(
        &self,
        region_id: Uuid,
        expected_count: usize,
    ) -> Result<Uuid, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let batch_id = self
            .infra
            .create_batch(&database_url, region_id, expected_count as i32)
            .await
            .map_err(ServiceError::InfraError)?;

        // Invalidate pipeline stats and per-status caches
        invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;
        invalidate_pattern(self.infra.as_ref(), &cache_keys::batches_status_pattern()).await;
        invalidate(self.infra.as_ref(), &cache_keys::region_status(region_id)).await;

        Ok(batch_id)
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

        let url = format!(
            "{}/fetcher-be/api/queue/enqueue",
            fetcher_url.trim_end_matches('/')
        );

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

    async fn add_tasks_to_batch(
        &self,
        batch_id: Uuid,
        task_ids: Vec<i64>,
    ) -> Result<(), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .add_tasks_to_batch(&database_url, batch_id, task_ids)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn update_batch_expected_count(
        &self,
        batch_id: Uuid,
        count: i32,
    ) -> Result<(), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .update_batch_expected_count(&database_url, batch_id, count)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn get_batch_by_id(
        &self,
        batch_id: Uuid,
    ) -> Result<Option<domain::ProcessingBatch>, Self::Error> {
        let infra = &self.infra;
        cached_or_fetch(
            infra.as_ref(),
            &cache_keys::batch_status(batch_id),
            cache_keys::TTL_SHORT,
            || async {
                let database_url = infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;

                infra
                    .get_batch_by_id(&database_url, batch_id)
                    .await
                    .map_err(ServiceError::InfraError)
            },
        )
        .await
    }

    async fn count_completed_tasks(&self, task_ids: Vec<i64>) -> Result<i32, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let count = self
            .infra
            .count_completed_tasks(&database_url, &task_ids)
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(count as i32)
    }

    async fn get_completed_task_ids(&self, task_ids: Vec<i64>) -> Result<Vec<i64>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_completed_task_ids(&database_url, &task_ids)
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
        let worker_status_url = format!(
            "{}/fetcher-be/api/queue/workers/status",
            fetcher_url.trim_end_matches('/')
        );

        tracing::debug!(url = %worker_status_url, "Checking worker status");

        let worker_status: WorkerStatusResponse = self
            .infra
            .get(&worker_status_url)
            .await
            .map_err(ServiceError::InfraError)?;

        // Count active workers
        let active_workers = worker_status
            .workers
            .iter()
            .filter(|w| w.status == "running")
            .count();

        tracing::info!(active_workers = active_workers, "Current worker count");

        // If no workers are active, allocate default number
        if active_workers == 0 {
            let database_url = self
                .infra
                .get_env_var("DATABASE_URL")
                .map_err(ServiceError::InfraError)?;

            let default_worker_count: u32 = self
                .infra
                .get_config(&database_url, ConfigKey::DefaultWorkerCount)
                .await
                .map_err(ServiceError::InfraError)?
                .and_then(|s| s.parse().ok())
                .unwrap_or(2); // Fallback to 2 if not configured

            tracing::info!(
                worker_count = default_worker_count,
                "No workers active, allocating default workers"
            );

            let allocate_url = format!(
                "{}/fetcher-be/api/queue/workers/allocate",
                fetcher_url.trim_end_matches('/')
            );

            // Read retry configuration from orch_config
            let retry_config = self.build_retry_config_from_db(&database_url).await?;

            let task_timeout_secs: u64 = self
                .infra
                .get_config(&database_url, ConfigKey::FetcherTaskTimeoutSecs)
                .await
                .map_err(ServiceError::InfraError)?
                .and_then(|s| s.parse().ok())
                .unwrap_or(2);

            let max_retry_attempts: u32 = self
                .infra
                .get_config(&database_url, ConfigKey::FetcherMaxRetryAttempts)
                .await
                .map_err(ServiceError::InfraError)?
                .and_then(|s| s.parse().ok())
                .unwrap_or(3);

            let request = AllocateWorkersRequest {
                worker_count: default_worker_count,
                task_timeout_secs,
                max_retry_attempts,
                retry_config: Some(retry_config),
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
            tracing::info!(
                active_workers = active_workers,
                "Workers already active, skipping allocation"
            );
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl<E, I> app::WorkerManagement for OrchBatchOrchestration<I>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E>
        + HttpClient<Error = E>
        + BatchManagement<Error = E>
        + crate::OrchDatabase<Error = E>
        + CacheClient<Error = E>
        + Send
        + Sync,
{
    type Error = ServiceError<E>;

    async fn get_worker_status(&self) -> Result<Vec<domain::WorkerStatus>, Self::Error> {
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

        let url = format!(
            "{}/fetcher-be/api/queue/workers/status",
            fetcher_url.trim_end_matches('/')
        );

        tracing::debug!(url = %url, "Getting worker status");

        let response: WorkerStatusResponse = self
            .infra
            .get(&url)
            .await
            .map_err(ServiceError::InfraError)?;

        // Transform proto-style response to domain types
        let workers = response
            .workers
            .into_iter()
            .map(|w| domain::WorkerStatus {
                worker_id: w.worker_id,
                status: w.status,
                current_task: w.current_task,
                tasks_processed: w.tasks_processed,
                started_at: w.started_at,
                worker_version: w.worker_version,
                last_heartbeat_at: w.last_heartbeat_at,
                uptime_seconds: w.uptime_seconds,
                tasks_failed: w.tasks_failed,
                success_rate: w.success_rate,
            })
            .collect();

        Ok(workers)
    }

    async fn allocate_workers(
        &self,
        req: domain::AllocateWorkersRequest,
    ) -> Result<domain::WorkerAllocationResponse, Self::Error> {
        // Validate request
        if req.worker_count == 0 {
            return Err(ServiceError::InvalidInput {
                message: "worker_count must be greater than 0".to_string(),
            });
        }

        if req.task_timeout_secs == 0 {
            return Err(ServiceError::InvalidInput {
                message: "task_timeout_secs must be greater than 0".to_string(),
            });
        }

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

        let url = format!(
            "{}/fetcher-be/api/queue/workers/allocate",
            fetcher_url.trim_end_matches('/')
        );

        // If user didn't provide retry config, load from DB
        let retry_config = match req.retry_config {
            Some(rc) => Some(rc),
            None => {
                let database_url = self
                    .infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;
                Some(self.build_retry_config_from_db(&database_url).await?)
            }
        };

        let request = AllocateWorkersRequest {
            worker_count: req.worker_count,
            task_timeout_secs: req.task_timeout_secs,
            max_retry_attempts: req.max_retry_attempts,
            retry_config,
        };

        tracing::info!(
            url = %url,
            worker_count = req.worker_count,
            timeout_secs = req.task_timeout_secs,
            "Allocating workers"
        );

        let response: AllocateWorkersResponse = self
            .infra
            .post(&url, &request)
            .await
            .map_err(ServiceError::InfraError)?;

        tracing::info!(
            success = response.success,
            worker_ids = ?response.worker_ids,
            "Worker allocation response"
        );

        Ok(domain::WorkerAllocationResponse {
            success: response.success,
            worker_ids: response.worker_ids,
            error_message: if response.error_message.is_empty() {
                None
            } else {
                Some(response.error_message)
            },
        })
    }

    async fn stop_workers(
        &self,
        req: domain::StopWorkersRequest,
    ) -> Result<domain::WorkerStopResponse, Self::Error> {
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

        let url = format!(
            "{}/fetcher-be/api/queue/workers/stop",
            fetcher_url.trim_end_matches('/')
        );

        #[derive(Serialize)]
        struct StopWorkersRequest {
            worker_ids: Vec<String>,
        }

        #[derive(Deserialize)]
        struct StopWorkersResponse {
            success: bool,
            workers_stopped: u32,
            error_message: String,
        }

        let request = StopWorkersRequest {
            worker_ids: req.worker_ids.clone(),
        };

        tracing::info!(
            url = %url,
            worker_ids = ?req.worker_ids,
            stop_all = req.worker_ids.is_empty(),
            "Stopping workers"
        );

        let response: StopWorkersResponse = self
            .infra
            .post(&url, &request)
            .await
            .map_err(ServiceError::InfraError)?;

        tracing::info!(
            success = response.success,
            workers_stopped = response.workers_stopped,
            "Worker stop response"
        );

        Ok(domain::WorkerStopResponse {
            success: response.success,
            workers_stopped: response.workers_stopped,
            error_message: if response.error_message.is_empty() {
                None
            } else {
                Some(response.error_message)
            },
        })
    }
}
