use crate::{
    BatchManagement, CacheClient, EnvInfra, HttpClient, OrchDatabase, RegionMappingQueries,
    ServiceError,
};
use app::PipelineRunner;
use backon::{ExponentialBuilder, Retryable};
use domain::ConfigKey;
use futures::StreamExt;
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

        // Read concurrency limit from config (reuses max_parallel_process_calls)
        let concurrency: usize = self
            .infra
            .get_config(&database_url, ConfigKey::MaxParallelProcessCalls)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        tracing::info!(concurrency, "Phase 1: Running parallel query generation");

        let regions_processed = Arc::new(AtomicUsize::new(0));
        let total_queries = Arc::new(AtomicUsize::new(0));
        let consecutive_failures = Arc::new(AtomicUsize::new(0));
        let circuit_broken = Arc::new(AtomicBool::new(false));
        const MAX_CONSECUTIVE_FAILURES: usize = 5;

        let infra = &self.infra;
        let url_ref = &url;
        let database_url_ref = &database_url;
        let rp = &regions_processed;
        let tq = &total_queries;
        let cf = &consecutive_failures;
        let cb = &circuit_broken;

        futures::stream::iter(regions)
            .map(|region| async move {
                // Check circuit breaker before starting
                if cb.load(Ordering::Relaxed) {
                    return;
                }

                tracing::info!(
                    region_id = %region.id,
                    region_name = %region.name,
                    "Phase 1: Generating queries for region"
                );

                let request = crate::GenerateQueriesRequest {
                    region_name: region.name.clone(),
                    count: query_count,
                };

                // Retry with exponential backoff (max 3 attempts, 1-10s delay)
                let retry_strategy = ExponentialBuilder::default()
                    .with_max_times(2)
                    .with_min_delay(std::time::Duration::from_secs(1))
                    .with_max_delay(std::time::Duration::from_secs(10));

                let req_ref = &request;

                let result = (|| async {
                    infra
                        .post::<crate::GenerateQueriesRequest, crate::GenerateQueriesResponse>(
                            url_ref, req_ref,
                        )
                        .await
                })
                .retry(retry_strategy)
                .notify(|err, dur: std::time::Duration| {
                    tracing::warn!(
                        region_id = %region.id,
                        error = %err,
                        retry_after_ms = dur.as_millis() as u64,
                        "Phase 1: Retrying query generation"
                    );
                })
                .await;

                match result {
                    Ok(response) => {
                        cf.store(0, Ordering::Relaxed); // Reset on success
                        if !response.queries.is_empty() {
                            match infra
                                .insert_queries(
                                    database_url_ref,
                                    region.id,
                                    response.queries.clone(),
                                )
                                .await
                            {
                                Ok(_) => {
                                    tq.fetch_add(response.queries.len(), Ordering::Relaxed);
                                    rp.fetch_add(1, Ordering::Relaxed);
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
                        let prev = cf.fetch_add(1, Ordering::Relaxed);
                        tracing::error!(
                            region_id = %region.id,
                            error = %e,
                            consecutive_failures = prev + 1,
                            "Phase 1: Failed to generate queries"
                        );
                        if prev + 1 >= MAX_CONSECUTIVE_FAILURES {
                            cb.store(true, Ordering::Relaxed);
                            tracing::error!(
                                "Phase 1: Circuit breaker tripped — aborting query generation. \
                                 Failed regions will retry next pipeline cycle."
                            );
                        }
                    }
                }
            })
            .buffer_unordered(concurrency)
            .collect::<Vec<()>>()
            .await;

        Ok((
            regions_processed.load(Ordering::Relaxed),
            total_queries.load(Ordering::Relaxed),
        ))
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

        // Read summary staleness window. Regions whose latest active summary is
        // younger than this are considered fresh and skipped by Phase 2 — the
        // background pipeline will not regenerate work that's still valid.
        // Manual `/api/regions/:id/generate` ignores this gate.
        let staleness_days: i64 = self
            .infra
            .get_config(&database_url, ConfigKey::SummaryStalenessDays)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let stale_cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::days(staleness_days);

        let mut regions_scanned = 0usize;
        let mut regions_skipped_fresh = 0usize;
        let mut total_new_tasks = 0usize;

        // For each region, re-run all queries against NCBI
        // UNIQUE(pmc_id) constraint deduplicates — only genuinely new papers get tasks
        for (region_id, region_name, queries) in &regions_with_queries {
            // Staleness gate: if the region already has a recent non-empty
            // summary, skip Phase 2 work entirely.
            match self
                .infra
                .get_latest_active_summary_age(&database_url, *region_id)
                .await
            {
                Ok(Some(age)) if age >= stale_cutoff => {
                    regions_skipped_fresh += 1;
                    tracing::debug!(
                        region_id = %region_id,
                        region_name = %region_name,
                        last_summary = %age,
                        "Phase 2: Skipping fresh region (within staleness window)"
                    );
                    continue;
                }
                Ok(_) => { /* stale or no summary -> proceed */ }
                Err(e) => {
                    tracing::warn!(
                        region_id = %region_id,
                        error = %e,
                        "Phase 2: Failed to read summary age, proceeding without staleness gate"
                    );
                }
            }

            let mut region_task_ids = Vec::new();

            for query in queries {
                let request = serde_json::json!({
                    "query": query,
                    "page_size": page_size,
                    "max_retry_attempts": max_retry_attempts
                });

                // Retry with exponential backoff (max 3 attempts, 1-10s delay)
                let retry_strategy = ExponentialBuilder::default()
                    .with_max_times(2)
                    .with_min_delay(std::time::Duration::from_secs(1))
                    .with_max_delay(std::time::Duration::from_secs(10));

                let infra = &self.infra;
                let enqueue_url_ref = &enqueue_url;
                let request_ref = &request;

                let result = (|| async {
                    infra
                        .post::<serde_json::Value, serde_json::Value>(enqueue_url_ref, request_ref)
                        .await
                })
                .retry(retry_strategy)
                .notify(|err, dur: std::time::Duration| {
                    tracing::warn!(
                        region_id = %region_id,
                        query = %query,
                        error = %err,
                        retry_after_ms = dur.as_millis() as u64,
                        "Phase 2: Retrying enqueue"
                    );
                })
                .await;

                match result {
                    Ok(response) => {
                        let task_ids: Vec<i64> = response
                            .get("task_ids")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
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

            // If we got new tasks, create a batch for them.
            // Deduplicate: ON CONFLICT(pmc_id) DO NOTHING in the fetcher returns
            // the same existing task ID for any paper found by multiple queries
            // for this region. Without dedup, the array grows with duplicates and
            // the completion watcher's count-based check never matches.
            let unique_task_ids: Vec<i64> = {
                let mut seen = std::collections::HashSet::new();
                region_task_ids
                    .iter()
                    .copied()
                    .filter(|id| seen.insert(*id))
                    .collect()
            };
            let region_task_ids = unique_task_ids;
            if !region_task_ids.is_empty() {
                // Check if there's already an active batch for this region
                match self.infra.get_active_batch(&database_url, *region_id).await {
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
                            .create_batch(&database_url, *region_id, region_task_ids.len() as i32)
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

        if regions_skipped_fresh > 0 {
            tracing::info!(
                regions_scanned,
                regions_skipped_fresh,
                staleness_days,
                "Phase 2: Staleness gate skipped fresh regions"
            );
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
            tracing::debug!(active_workers, "Phase 3: Workers already running");
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

    async fn get_system_stats(&self) -> Result<domain::SystemStats, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let raw = self
            .infra
            .get_system_stats(&database_url)
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(domain::SystemStats {
            fetch_tasks_by_status: raw
                .fetch_tasks_by_status
                .into_iter()
                .map(|(s, c)| domain::StatusCount {
                    status: s,
                    count: c,
                })
                .collect(),
            batches_by_status: raw
                .batches_by_status
                .into_iter()
                .map(|(s, c)| domain::StatusCount {
                    status: s,
                    count: c,
                })
                .collect(),
            total_queries: raw.total_queries,
            regions_with_queries: raw.regions_with_queries,
            query_distribution: raw
                .query_distribution
                .into_iter()
                .map(|(q, n)| domain::QueryDistEntry {
                    query_count: q,
                    num_regions: n,
                })
                .collect(),
            total_papers: raw.total_papers,
            total_summaries: raw.total_summaries,
            timestamp: chrono::Utc::now(),
        })
    }

    async fn get_redis_stats(&self) -> Result<domain::RedisStats, Self::Error> {
        // The CacheClient impl is designed to never propagate errors -- a Redis
        // outage produces a `connected: false` snapshot. So this is a thin
        // delegation.
        self.infra
            .cache_stats()
            .await
            .map_err(ServiceError::InfraError)
    }
}
