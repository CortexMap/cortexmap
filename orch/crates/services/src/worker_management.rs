use crate::{EnvInfra, HttpClient, ServiceError};
use app::WorkerManagement;
use domain::{
    AllocateWorkersRequest, AllocateWorkersResult, ConfigKey, StopWorkersRequest,
    StopWorkersResult, WorkerStatusResult,
};
use std::error::Error;
use std::sync::Arc;

pub struct OrchWorkerManagement<I> {
    infra: Arc<I>,
}

impl<I> OrchWorkerManagement<I> {
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

#[async_trait::async_trait]
impl<E, I> WorkerManagement for OrchWorkerManagement<I>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E> + HttpClient<Error = E> + crate::OrchDatabase<Error = E> + Send + Sync,
{
    type Error = ServiceError<E>;

    async fn get_worker_status(&self) -> Result<WorkerStatusResult, Self::Error> {
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
            "{}/fetcher-be/api/queue/workers/status",
            fetcher_url.trim_end_matches('/')
        );

        tracing::debug!(url = %url, "Getting worker status");

        self.infra
            .get(&url)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn allocate_workers(
        &self,
        request: AllocateWorkersRequest,
    ) -> Result<AllocateWorkersResult, Self::Error> {
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
            "{}/fetcher-be/api/queue/workers/allocate",
            fetcher_url.trim_end_matches('/')
        );

        tracing::info!(
            url = %url,
            worker_count = request.worker_count,
            "Allocating workers"
        );

        self.infra
            .post(&url, &request)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn stop_workers(
        &self,
        request: StopWorkersRequest,
    ) -> Result<StopWorkersResult, Self::Error> {
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
            "{}/fetcher-be/api/queue/workers/stop",
            fetcher_url.trim_end_matches('/')
        );

        tracing::info!(
            url = %url,
            worker_ids = ?request.worker_ids,
            "Stopping workers"
        );

        self.infra
            .post(&url, &request)
            .await
            .map_err(ServiceError::InfraError)
    }
}
