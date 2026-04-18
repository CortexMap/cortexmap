use crate::{
    BatchManagement, CacheClient, EnvInfra, HttpClient, OrchDatabase, RegionMappingQueries,
    ServiceError,
};
use app::PipelineRunner;
use domain::ConfigKey;
use std::error::Error;
use std::sync::Arc;

pub struct OrchPipelineRunner<I> {
    infra: Arc<I>,
}

impl<I> OrchPipelineRunner<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<E, I> PipelineRunner for OrchPipelineRunner<I>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E>
        + HttpClient<Error = E>
        + BatchManagement<Error = E>
        + OrchDatabase<Error = E>
        + RegionMappingQueries<Error = E>
        + CacheClient<Error = E>
        + Send
        + Sync,
{
    type Error = ServiceError<E>;

    async fn generate_queries_for_new_regions(&self) -> Result<(usize, usize), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        // Get regions that have no queries yet
        let regions = self
            .infra
            .get_regions_without_queries(&database_url)
            .await
            .map_err(ServiceError::InfraError)?;

        if regions.is_empty() {
            tracing::debug!("Phase 1: All regions already have queries");
            return Ok((0, 0));
        }

        tracing::info!(
            count = regions.len(),
            "Phase 1: Found regions needing query generation"
        );

        // Get query count from config
        let query_count: u32 = self
            .infra
            .get_config(&database_url, ConfigKey::QueryGenerationLimit)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        // Get brainatlas URL
        fn normalize_url(addr: &str) -> String {
            if addr.starts_with("http://") || addr.starts_with("https://") {
                addr.to_string()
            } else {
                let replaced = addr.replace("0.0.0.0", "localhost");
                format!("http://{}", replaced)
            }
        }

        let brainatlas_url = match self.infra.get_env_var("BRAINATLAS_HTTP_ADDR") {
            Ok(addr) => normalize_url(&addr),
            Err(_) => self
                .infra
                .get_config(&database_url, ConfigKey::BrainatlasBaseUrl)
                .await
                .map_err(ServiceError::InfraError)?
                .ok_or_else(|| ServiceError::ConfigNotFound {
                    key: "brainatlas_base_url".to_string(),
                })?,
        };

        let url = format!(
            "{}/brainatlas-be/api/generate-queries",
            brainatlas_url.trim_end_matches('/')
        );

        let mut regions_processed = 0usize;
        let mut total_queries = 0usize;

        // Process regions one at a time (sequential to respect LLM rate limits)
        for region in &regions {
            tracing::info!(
                region_id = %region.id,
                region_name = %region.name,
                "Phase 1: Generating queries for region"
            );

            let request = crate::GenerateQueriesRequest {
                region_name: region.name.clone(),
                count: query_count,
            };

            match self
                .infra
                .post::<crate::GenerateQueriesRequest, crate::GenerateQueriesResponse>(
                    &url, &request,
                )
                .await
            {
                Ok(response) => {
                    if !response.queries.is_empty() {
                        match self
                            .infra
                            .insert_queries(&database_url, region.id, response.queries.clone())
                            .await
                        {
                            Ok(_) => {
                                total_queries += response.queries.len();
                                regions_processed += 1;
                                tracing::info!(
                                    region_id = %region.id,
                                    region_name = %region.name,
                                    queries = response.queries.len(),
                                    "Phase 1: Queries generated and stored"
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    region_id = %region.id,
                                    error = %e,
                                    "Phase 1: Failed to store queries"
                                );
                            }
                        }
                    } else {
                        tracing::warn!(
                            region_id = %region.id,
                            "Phase 1: LLM returned empty queries"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        region_id = %region.id,
                        error = %e,
                        "Phase 1: Failed to generate queries"
                    );
                }
            }
        }

        Ok((regions_processed, total_queries))
    }

    async fn discover_new_papers(&self) -> Result<(usize, usize), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        // Get ALL regions that have queries
        let regions_with_queries = self
            .infra
            .get_all_regions_with_queries(&database_url)
            .await
            .map_err(ServiceError::InfraError)?;

        if regions_with_queries.is_empty() {
            tracing::debug!("Phase 2: No regions with queries found");
            return Ok((0, 0));
        }

        tracing::info!(
            count = regions_with_queries.len(),
            "Phase 2: Scanning all regions for new papers"
        );

        // Get fetcher URL
        fn normalize_url(addr: &str) -> String {
            if addr.starts_with("http://") || addr.starts_with("https://") {
                addr.to_string()
            } else {
                let replaced = addr.replace("0.0.0.0", "localhost");
                format!("http://{}", replaced)
            }
        }

        let fetcher_url = match self.infra.get_env_var("FETCHER_HTTP_ADDR") {
            Ok(addr) => normalize_url(&addr),
            Err(_) => self
                .infra
                .get_config(&database_url, ConfigKey::FetcherBaseUrl)
                .await
                .map_err(ServiceError::InfraError)?
                .ok_or_else(|| ServiceError::ConfigNotFound {
                    key: "fetcher_base_url".to_string(),
                })?,
        };

        let enqueue_url = format!(
            "{}/fetcher-be/api/queue/enqueue",
            fetcher_url.trim_end_matches('/')
        );

        // Read page_size and max_retry_attempts from config instead of hardcoding
        let page_size: u32 = self
            .infra
            .get_config(&database_url, ConfigKey::EnqueuePageSize)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);

        let max_retry_attempts: u32 = self
            .infra
            .get_config(&database_url, ConfigKey::FetcherMaxRetryAttempts)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        let mut regions_scanned = 0usize;
        let mut total_new_tasks = 0usize;

        // For each region, re-run all queries against NCBI
        // UNIQUE(pmc_id) constraint deduplicates — only genuinely new papers get tasks
        for (region_id, region_name, queries) in &regions_with_queries {
            let mut region_task_ids = Vec::new();

            for query in queries {
                let request = serde_json::json!({
                    "query": query,
                    "page_size": page_size,
                    "max_retry_attempts": max_retry_attempts
                });

                match self
                    .infra
                    .post::<serde_json::Value, serde_json::Value>(&enqueue_url, &request)
                    .await
                {
                    Ok(response) => {
                        let task_ids: Vec<i64> = response
                            .get("task_ids")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter().filter_map(|v| v.as_i64()).collect()
                            })
                            .unwrap_or_default();

                        if !task_ids.is_empty() {
                            region_task_ids.extend(task_ids);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            region_id = %region_id,
                            query = %query,
                            error = %e,
                            "Phase 2: Failed to enqueue query"
                        );
                    }
                }
            }

            // If we got new tasks, create a batch for them
            if !region_task_ids.is_empty() {
                // Check if there's already an active batch for this region
                match self
                    .infra
                    .get_active_batch(&database_url, *region_id)
                    .await
                {
                    Ok(Some(existing_batch)) => {
                        tracing::debug!(
                            region_id = %region_id,
                            batch_id = %existing_batch.id,
                            "Phase 2: Active batch already exists, skipping batch creation"
                        );
                    }
                    Ok(None) => {
                        // Create a new batch
                        match self
                            .infra
                            .create_batch(
                                &database_url,
                                *region_id,
                                region_task_ids.len() as i32,
                            )
                            .await
                        {
                            Ok(batch_id) => {
                                if let Err(e) = self
                                    .infra
                                    .add_tasks_to_batch(
                                        &database_url,
                                        batch_id,
                                        region_task_ids.clone(),
                                    )
                                    .await
                                {
                                    tracing::error!(
                                        batch_id = %batch_id,
                                        error = %e,
                                        "Phase 2: Failed to add tasks to batch"
                                    );
                                } else {
                                    total_new_tasks += region_task_ids.len();
                                    tracing::info!(
                                        region_id = %region_id,
                                        region_name = %region_name,
                                        batch_id = %batch_id,
                                        tasks = region_task_ids.len(),
                                        "Phase 2: Created batch with new tasks"
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    region_id = %region_id,
                                    error = %e,
                                    "Phase 2: Failed to create batch"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            region_id = %region_id,
                            error = %e,
                            "Phase 2: Failed to check active batch"
                        );
                    }
                }
            }

            regions_scanned += 1;
        }

        Ok((regions_scanned, total_new_tasks))
    }

    async fn ensure_fetcher_running(&self) -> Result<(), Self::Error> {
        // Delegate to the existing ensure_workers_allocated pattern
        fn normalize_url(addr: &str) -> String {
            if addr.starts_with("http://") || addr.starts_with("https://") {
                addr.to_string()
            } else {
                let replaced = addr.replace("0.0.0.0", "localhost");
                format!("http://{}", replaced)
            }
        }

        let fetcher_url = match self.infra.get_env_var("FETCHER_HTTP_ADDR") {
            Ok(addr) => normalize_url(&addr),
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

        #[derive(serde::Deserialize)]
        struct WorkerStatusResponse {
            workers: Vec<WorkerInfo>,
        }

        #[derive(serde::Deserialize)]
        struct WorkerInfo {
            #[allow(dead_code)]
            worker_id: String,
            status: String,
        }

        let worker_status: WorkerStatusResponse = self
            .infra
            .get(&worker_status_url)
            .await
            .map_err(ServiceError::InfraError)?;

        let active_workers = worker_status
            .workers
            .iter()
            .filter(|w| w.status == "running")
            .count();

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
                .unwrap_or(2);

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

            let allocate_url = format!(
                "{}/fetcher-be/api/queue/workers/allocate",
                fetcher_url.trim_end_matches('/')
            );

            let request = serde_json::json!({
                "worker_count": default_worker_count,
                "task_timeout_secs": task_timeout_secs,
                "max_retry_attempts": max_retry_attempts
            });

            let _: serde_json::Value = self
                .infra
                .post(&allocate_url, &request)
                .await
                .map_err(ServiceError::InfraError)?;

            tracing::info!(
                worker_count = default_worker_count,
                "Phase 3: Allocated workers"
            );
        } else {
            tracing::debug!(
                active_workers,
                "Phase 3: Workers already running"
            );
        }

        Ok(())
    }

    async fn get_pending_fetch_task_count(&self) -> Result<i64, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_pending_fetch_task_count(&database_url)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn generate_queries_for_new_regions_count(&self) -> Result<i64, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let regions = self
            .infra
            .get_regions_without_queries(&database_url)
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(regions.len() as i64)
    }

    async fn get_regions_with_queries_count(&self) -> Result<i64, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let regions = self
            .infra
            .get_all_regions_with_queries(&database_url)
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(regions.len() as i64)
    }
}
