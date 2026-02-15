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
    ) -> Result<i64, Self::Error> {
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

        // Return the first task ID (or 0 if none)
        // The app layer will collect all task IDs from multiple queries
        Ok(response.task_ids.first().copied().unwrap_or(0))
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
}
