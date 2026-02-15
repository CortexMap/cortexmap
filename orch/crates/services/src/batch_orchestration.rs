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
}

#[derive(Debug, Serialize)]
struct EnqueueRequest {
    query: String,
    region_id: Option<Uuid>,
    priority: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct EnqueueResponse {
    task_id: i64,
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
        priority: i32,
    ) -> Result<i64, Self::Error> {
        // Try env var first, fall back to config
        let fetcher_url = match self.infra.get_env_var("FETCHER_URL") {
            Ok(url) => url,
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

        let url = format!("{}/api/queue/enqueue", fetcher_url.trim_end_matches('/'));

        let request = EnqueueRequest {
            query,
            region_id: Some(region_id),
            priority: Some(priority),
        };

        let response: EnqueueResponse = self
            .infra
            .post(&url, &request)
            .await
            .map_err(ServiceError::InfraError)?;

        tracing::info!(task_id = response.task_id, region_id = %region_id, "Enqueued fetch task");

        Ok(response.task_id)
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
