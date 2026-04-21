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

/// Helpers shared across the `PipelineRunner` impl. Kept separate so the
/// trait impl isn't cluttered with private utilities and so we can call
/// them with a different bound set than the trait requires.
impl<E, I> OrchPipelineRunner<I>
where
    E: Error + Send + Sync + 'static,
    I: HttpClient<Error = E> + Send + Sync,
{
    /// POST the knowledge-only summary request to brainatlas, with an
    /// exponential retry. Returns `Ok(())` on success; error is wrapped in
    /// `ServiceError` on persistent failure.
    async fn generate_knowledge_summary(
        &self,
        url: &str,
        region_id: uuid::Uuid,
        batch_id: uuid::Uuid,
        region_name: &str,
    ) -> Result<(), ServiceError<E>> {
        let request = crate::ProcessNoPapersRequest {
            region_id: crate::UuidWrapper {
                value: region_id.to_string(),
            },
            batch_id: crate::UuidWrapper {
                value: batch_id.to_string(),
            },
            chat_model: None,
            correlation_id: Some(format!("batch:{}", batch_id)),
        };

        let retry_strategy = ExponentialBuilder::default()
            .with_max_times(2)
            .with_min_delay(std::time::Duration::from_secs(2))
            .with_max_delay(std::time::Duration::from_secs(30));

        let infra = &self.infra;
        let req_ref = &request;
        let url_ref = url;

        let _: crate::ProcessRegionResponse = (|| async {
            infra
                .post::<crate::ProcessNoPapersRequest, crate::ProcessRegionResponse>(
                    url_ref, req_ref,
                )
                .await
        })
        .retry(retry_strategy)
        .notify(|err, dur: std::time::Duration| {
            tracing::warn!(
                region_id = %region_id,
                region_name = %region_name,
                batch_id = %batch_id,
                error = %err,
                retry_after_ms = dur.as_millis() as u64,
                "Phase 2: Retrying knowledge-only summary POST"
            );
        })
        .await
        .map_err(ServiceError::InfraError)?;

        Ok(())
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
                    correlation_id: Some(format!("region:{}", region.id)),
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

        // Also resolve the brainatlas URL — needed for the knowledge-only
        // summary path (regions whose NCBI queries return zero papers).
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
        let knowledge_summary_url = format!(
            "{}/brainatlas-be/api/process-no-papers",
            brainatlas_url.trim_end_matches('/')
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
        let mut knowledge_summaries_attempted = 0usize;
        let mut knowledge_summaries_succeeded = 0usize;

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
            } else {
                // No papers found for any of this region's queries. Fall back
                // to a knowledge-only summary so every region ends up with at
                // least one entry in `region_summary`.
                //
                // Only attempt this if there's no active batch already — an
                // in-flight batch (e.g. from a prior cycle where NCBI did
                // return results) should run to completion on its own.
                match self.infra.get_active_batch(&database_url, *region_id).await {
                    Ok(Some(existing_batch)) => {
                        tracing::debug!(
                            region_id = %region_id,
                            batch_id = %existing_batch.id,
                            "Phase 2: No new papers, but an active batch exists — skipping knowledge-only fallback"
                        );
                    }
                    Ok(None) => {
                        knowledge_summaries_attempted += 1;
                        match self.infra.create_batch(&database_url, *region_id, 0).await {
                            Ok(batch_id) => {
                                // Mark the batch Processing so it's visible
                                // to the dashboards and zombie-watcher.
                                if let Err(e) = self
                                    .infra
                                    .update_batch_status(
                                        &database_url,
                                        batch_id,
                                        domain::BatchStatus::Processing,
                                        None,
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        batch_id = %batch_id,
                                        error = %e,
                                        "Phase 2: Failed to mark knowledge-only batch as Processing"
                                    );
                                }

                                match self
                                    .generate_knowledge_summary(
                                        &knowledge_summary_url,
                                        *region_id,
                                        batch_id,
                                        region_name,
                                    )
                                    .await
                                {
                                    Ok(()) => {
                                        if let Err(e) =
                                            self.infra.complete_batch(&database_url, batch_id).await
                                        {
                                            tracing::warn!(
                                                batch_id = %batch_id,
                                                error = %e,
                                                "Phase 2: Knowledge summary succeeded but marking batch complete failed"
                                            );
                                        }
                                        knowledge_summaries_succeeded += 1;
                                        tracing::info!(
                                            region_id = %region_id,
                                            region_name = %region_name,
                                            batch_id = %batch_id,
                                            "Phase 2: Generated knowledge-only summary (zero NCBI results)"
                                        );
                                    }
                                    Err(e) => {
                                        let err_msg = e.to_string();
                                        if let Err(upd) = self
                                            .infra
                                            .update_batch_status(
                                                &database_url,
                                                batch_id,
                                                domain::BatchStatus::Failed,
                                                Some(err_msg.clone()),
                                            )
                                            .await
                                        {
                                            tracing::warn!(
                                                batch_id = %batch_id,
                                                error = %upd,
                                                "Phase 2: Failed to mark knowledge-only batch Failed"
                                            );
                                        }
                                        tracing::error!(
                                            region_id = %region_id,
                                            region_name = %region_name,
                                            batch_id = %batch_id,
                                            error = %err_msg,
                                            "Phase 2: Knowledge-only summary generation failed"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    region_id = %region_id,
                                    error = %e,
                                    "Phase 2: Failed to create knowledge-only batch"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            region_id = %region_id,
                            error = %e,
                            "Phase 2: Failed to check active batch for knowledge-only fallback"
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
        if knowledge_summaries_attempted > 0 {
            tracing::info!(
                knowledge_summaries_attempted,
                knowledge_summaries_succeeded,
                "Phase 2: Knowledge-only summary generation stats"
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

#[cfg(test)]
mod tests {
    //! Unit tests for `OrchPipelineRunner`.
    //!
    //! Scope notes / deliberately skipped branches:
    //!
    //! * The task plan mentions a `skip_summarization` flag and an
    //!   `AtomicBool` cancellation toggle. Neither lives in this file:
    //!   `skip_summarization` is a field of `ProcessRegionRequest` in
    //!   `types.rs` and is set by `completion_watcher.rs`; the only
    //!   `AtomicBool` here is the per-cycle `circuit_broken` flag, which
    //!   we exercise below via `generate_queries_circuit_breaker_trips_after_5_failures`.
    //! * `normalize_url` in `pipeline_runner.rs:70-78` and the twin in
    //!   `pipeline_runner.rs:256-263` are nested functions defined inside
    //!   `generate_queries_for_new_regions` / `discover_new_papers`. They
    //!   are unreachable from an external test module. We test their
    //!   behaviour indirectly by inspecting the URL the runner POSTs to
    //!   (`http_passthrough_*`, `rewrites_0_0_0_0_*`, `trailing_slash_*`).
    //! * `discover_new_papers`, `ensure_fetcher_running`, `get_system_stats`,
    //!   and `get_redis_stats` are out of scope for Task 1.4 (which is
    //!   specifically the pipeline-runner loop / query-generation phase).
    //!   Only the Phase-1 code path plus the lightweight count accessors
    //!   are covered here.

    use super::*;
    use crate::infra::{NewProcessedFetchTask, OrchConfig, ProcessedFetchTask};
    use crate::{
        ChunkSourceRecord, PaperMetadataRecord, RegionInfo, RegionMapping, RegionSummaryRecord,
        SearchHitRecord, SummaryFreshnessCounts, SystemStatsRaw,
    };
    use ::app::PipelineRunner as PipelineRunnerTrait;
    use async_trait::async_trait;
    use domain::{BatchStatus, ConfigKey, ProcessingBatch, RedisStats, RegionQuery};
    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    // ---- Error type ----

    #[derive(Debug, thiserror::Error)]
    #[error("mock error: {0}")]
    struct MockErr(String);

    // ---- Recorded HTTP call ----

    #[derive(Debug, Clone)]
    struct PostCall {
        url: String,
        body: serde_json::Value,
    }

    /// Per-call behaviour: given (call_index, url, body_json), produce a
    /// result. The counter lets a test make the same region fail on the
    /// first attempt and succeed on the retry.
    type PostResponder = Box<
        dyn Fn(usize, &str, &serde_json::Value) -> Result<serde_json::Value, MockErr> + Send + Sync,
    >;

    // ---- Mock infra ----

    struct MockInfra {
        env: HashMap<String, String>,
        config: Mutex<HashMap<String, String>>,
        regions_without_queries: Mutex<Vec<RegionInfo>>,
        /// Populated when `insert_queries` is called: (region_id, queries).
        inserted_queries: Mutex<Vec<(Uuid, Vec<String>)>>,
        post_calls: Mutex<Vec<PostCall>>,
        post_responder: Mutex<Option<PostResponder>>,
        /// Value returned by `get_pending_fetch_task_count`.
        pending_fetch_task_count: Mutex<i64>,
        /// Optional DB-level error for `get_regions_without_queries`.
        regions_without_queries_err: Mutex<Option<String>>,
    }

    impl MockInfra {
        fn new() -> Self {
            Self {
                env: HashMap::new(),
                config: Mutex::new(HashMap::new()),
                regions_without_queries: Mutex::new(Vec::new()),
                inserted_queries: Mutex::new(Vec::new()),
                post_calls: Mutex::new(Vec::new()),
                post_responder: Mutex::new(None),
                pending_fetch_task_count: Mutex::new(0),
                regions_without_queries_err: Mutex::new(None),
            }
        }
        fn with_env(mut self, k: &str, v: &str) -> Self {
            self.env.insert(k.to_string(), v.to_string());
            self
        }
        fn with_config(self, k: ConfigKey, v: &str) -> Self {
            self.config
                .lock()
                .unwrap()
                .insert(k.to_string(), v.to_string());
            self
        }
        fn with_regions(self, regions: Vec<RegionInfo>) -> Self {
            *self.regions_without_queries.lock().unwrap() = regions;
            self
        }
        fn with_post_responder(self, f: PostResponder) -> Self {
            *self.post_responder.lock().unwrap() = Some(f);
            self
        }
        fn with_pending_count(self, n: i64) -> Self {
            *self.pending_fetch_task_count.lock().unwrap() = n;
            self
        }
        fn with_regions_without_queries_err(self, msg: &str) -> Self {
            *self.regions_without_queries_err.lock().unwrap() = Some(msg.to_string());
            self
        }
    }

    fn region(name: &str) -> RegionInfo {
        RegionInfo {
            id: Uuid::new_v4(),
            name: name.to_string(),
        }
    }

    // ---- Trait impls ----

    impl EnvInfra for MockInfra {
        type Error = MockErr;
        fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
            self.env
                .get(key)
                .cloned()
                .ok_or_else(|| MockErr(format!("no env {}", key)))
        }
    }

    #[async_trait]
    impl OrchDatabase for MockInfra {
        type Error = MockErr;

        async fn get_config(
            &self,
            _database_url: &str,
            key: ConfigKey,
        ) -> Result<Option<String>, Self::Error> {
            Ok(self.config.lock().unwrap().get(&key.to_string()).cloned())
        }

        async fn get_processed_task(
            &self,
            _database_url: &str,
            _fetch_task_id: i64,
        ) -> Result<Option<ProcessedFetchTask>, Self::Error> {
            unimplemented!("not used by pipeline_runner tests")
        }
        async fn insert_processed_task(
            &self,
            _database_url: &str,
            _task: NewProcessedFetchTask,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner tests")
        }
        async fn update_brainatlas_status(
            &self,
            _database_url: &str,
            _fetch_task_id: i64,
            _status: &str,
            _error: Option<String>,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner tests")
        }
        async fn get_all_config(
            &self,
            _database_url: &str,
        ) -> Result<Vec<OrchConfig>, Self::Error> {
            unimplemented!("not used by pipeline_runner tests")
        }
        async fn update_config(
            &self,
            _database_url: &str,
            _key: ConfigKey,
            _value: &str,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner tests")
        }
    }

    #[async_trait]
    impl HttpClient for MockInfra {
        type Error = MockErr;

        async fn get<T: DeserializeOwned + Send>(&self, _url: &str) -> Result<T, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }

        async fn post<Req: Serialize + Send + Sync, Res: DeserializeOwned + Send + Sync>(
            &self,
            url: &str,
            body: &Req,
        ) -> Result<Res, Self::Error> {
            let body_json = serde_json::to_value(body)
                .map_err(|e| MockErr(format!("serialize body: {}", e)))?;
            let mut calls = self.post_calls.lock().unwrap();
            let idx = calls.len();
            calls.push(PostCall {
                url: url.to_string(),
                body: body_json.clone(),
            });
            drop(calls);

            let guard = self.post_responder.lock().unwrap();
            let responder = guard.as_ref().expect("test did not stage a post responder");
            let value = responder(idx, url, &body_json)?;
            serde_json::from_value(value).map_err(|e| MockErr(format!("deserialize: {}", e)))
        }

        async fn check_health(
            &self,
            _base_url: &str,
            _service_name: &str,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner tests")
        }
    }

    #[async_trait]
    impl BatchManagement for MockInfra {
        type Error = MockErr;

        async fn insert_queries(
            &self,
            _database_url: &str,
            region_id: Uuid,
            queries: Vec<String>,
        ) -> Result<Vec<Uuid>, Self::Error> {
            self.inserted_queries
                .lock()
                .unwrap()
                .push((region_id, queries.clone()));
            Ok(queries.iter().map(|_| Uuid::new_v4()).collect())
        }

        async fn get_active_batch(
            &self,
            _database_url: &str,
            _region_id: Uuid,
        ) -> Result<Option<ProcessingBatch>, Self::Error> {
            // Phase-1 code path never calls this; Phase-2 does. Returning
            // `Ok(None)` keeps the surface safe if a future test wires in
            // `discover_new_papers` without staging batch state.
            Ok(None)
        }

        async fn get_queries(
            &self,
            _database_url: &str,
            _region_id: Uuid,
        ) -> Result<Vec<RegionQuery>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn delete_queries(
            &self,
            _database_url: &str,
            _region_id: Uuid,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn delete_all_queries(&self, _database_url: &str) -> Result<i64, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn create_batch(
            &self,
            _database_url: &str,
            _region_id: Uuid,
            _expected_count: i32,
        ) -> Result<Uuid, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn add_tasks_to_batch(
            &self,
            _database_url: &str,
            _batch_id: Uuid,
            _task_ids: Vec<i64>,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn update_batch_expected_count(
            &self,
            _database_url: &str,
            _batch_id: Uuid,
            _count: i32,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_batch_by_id(
            &self,
            _database_url: &str,
            _batch_id: Uuid,
        ) -> Result<Option<ProcessingBatch>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_batches_by_status(
            &self,
            _database_url: &str,
            _status: BatchStatus,
        ) -> Result<Vec<ProcessingBatch>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn count_completed_tasks(
            &self,
            _database_url: &str,
            _task_ids: &[i64],
        ) -> Result<usize, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_completed_task_ids(
            &self,
            _database_url: &str,
            _task_ids: &[i64],
        ) -> Result<Vec<i64>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_task_s3_keys(
            &self,
            _database_url: &str,
            _task_ids: &[i64],
        ) -> Result<Vec<String>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_task_paper_metadata(
            &self,
            _database_url: &str,
            _task_ids: &[i64],
        ) -> Result<Vec<PaperMetadataRecord>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn update_batch_status(
            &self,
            _database_url: &str,
            _batch_id: Uuid,
            _status: BatchStatus,
            _error: Option<String>,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn complete_batch(
            &self,
            _database_url: &str,
            _batch_id: Uuid,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_recent_batch(
            &self,
            _database_url: &str,
            _region_id: Uuid,
        ) -> Result<Option<ProcessingBatch>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
    }

    #[async_trait]
    impl RegionMappingQueries for MockInfra {
        type Error = MockErr;

        async fn get_regions_without_queries(
            &self,
            _database_url: &str,
        ) -> Result<Vec<RegionInfo>, Self::Error> {
            if let Some(msg) = self.regions_without_queries_err.lock().unwrap().as_ref() {
                return Err(MockErr(msg.clone()));
            }
            Ok(self.regions_without_queries.lock().unwrap().clone())
        }

        async fn get_pending_fetch_task_count(
            &self,
            _database_url: &str,
        ) -> Result<i64, Self::Error> {
            Ok(*self.pending_fetch_task_count.lock().unwrap())
        }

        async fn get_region_mapping(
            &self,
            _database_url: &str,
            _region_uuid: Uuid,
        ) -> Result<Option<RegionMapping>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_all_regions(
            &self,
            _database_url: &str,
        ) -> Result<Vec<RegionMapping>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_total_region_count(&self, _database_url: &str) -> Result<i64, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn count_regions_without_batches(
            &self,
            _database_url: &str,
        ) -> Result<i64, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn count_actively_fetching_regions(
            &self,
            _database_url: &str,
        ) -> Result<i64, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_region_summaries(
            &self,
            _database_url: &str,
            _region_id: i32,
        ) -> Result<Vec<RegionSummaryRecord>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_summary_sources(
            &self,
            _database_url: &str,
            _summary_id: Uuid,
        ) -> Result<Vec<ChunkSourceRecord>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn search_regions(
            &self,
            _database_url: &str,
            _query: &str,
            _limit: i64,
        ) -> Result<(Vec<SearchHitRecord>, i64), Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_all_regions_with_queries(
            &self,
            _database_url: &str,
        ) -> Result<Vec<(Uuid, String, Vec<String>)>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_latest_active_summary_age(
            &self,
            _database_url: &str,
            _region_id: Uuid,
        ) -> Result<Option<chrono::NaiveDateTime>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_summary_freshness_counts(
            &self,
            _database_url: &str,
            _staleness_days: i64,
        ) -> Result<SummaryFreshnessCounts, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn get_system_stats(
            &self,
            _database_url: &str,
        ) -> Result<SystemStatsRaw, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
    }

    #[async_trait]
    impl CacheClient for MockInfra {
        type Error = MockErr;

        async fn cache_get(&self, _key: &str) -> Result<Option<String>, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn cache_set(
            &self,
            _key: &str,
            _value: &str,
            _ttl_secs: u64,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn cache_del(&self, _key: &str) -> Result<(), Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn cache_del_pattern(&self, _pattern: &str) -> Result<u64, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
        async fn cache_stats(&self) -> Result<RedisStats, Self::Error> {
            unimplemented!("not used by pipeline_runner Phase-1 tests")
        }
    }

    fn base_infra() -> MockInfra {
        MockInfra::new().with_env("DATABASE_URL", "postgres://mock")
    }

    /// Default responder that returns 2 queries for any region.
    fn ok_two_queries() -> PostResponder {
        Box::new(|_idx, _url, _body| {
            Ok(serde_json::json!({
                "queries": ["q1", "q2"],
            }))
        })
    }

    // ======================================================================
    //  generate_queries_for_new_regions
    // ======================================================================

    #[tokio::test]
    async fn generate_queries_empty_regions_short_circuits() {
        // No regions -> return early with (0, 0), never touch HTTP / config keys.
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_regions(vec![]),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        let (regions_processed, total_queries) =
            runner.generate_queries_for_new_regions().await.expect("ok");
        assert_eq!(regions_processed, 0);
        assert_eq!(total_queries, 0);
        // No HTTP POSTs must have been attempted.
        assert!(infra.post_calls.lock().unwrap().is_empty());
        // No queries were inserted.
        assert!(infra.inserted_queries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn generate_queries_uses_config_query_limit() {
        // When the config has a valid integer, `count` in the request body
        // should equal it.
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_config(ConfigKey::QueryGenerationLimit, "7")
                .with_regions(vec![region("R1")])
                .with_post_responder(ok_two_queries()),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        runner.generate_queries_for_new_regions().await.expect("ok");

        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].body["count"], serde_json::json!(7));
    }

    #[tokio::test]
    async fn generate_queries_defaults_query_limit_when_missing() {
        // No config -> default 3.
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_regions(vec![region("R1")])
                .with_post_responder(ok_two_queries()),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        runner.generate_queries_for_new_regions().await.expect("ok");

        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(calls[0].body["count"], serde_json::json!(3));
    }

    #[tokio::test]
    async fn generate_queries_defaults_query_limit_when_invalid_string() {
        // Unparseable config -> .parse().ok() is None -> default 3.
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_config(ConfigKey::QueryGenerationLimit, "not-a-number")
                .with_regions(vec![region("R1")])
                .with_post_responder(ok_two_queries()),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        runner.generate_queries_for_new_regions().await.expect("ok");

        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(calls[0].body["count"], serde_json::json!(3));
    }

    #[tokio::test]
    async fn generate_queries_honours_zero_query_limit() {
        // "0" parses fine; the request still goes out with count=0 and the
        // code path continues — the LLM can legitimately return [] or the
        // default. This documents the current behaviour (no lower-bound
        // clamp on QueryGenerationLimit).
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_config(ConfigKey::QueryGenerationLimit, "0")
                .with_regions(vec![region("R1")])
                .with_post_responder(ok_two_queries()),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        runner.generate_queries_for_new_regions().await.expect("ok");

        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(calls[0].body["count"], serde_json::json!(0));
    }

    #[tokio::test]
    async fn generate_queries_http_passthrough_url() {
        // The env var already has an http:// scheme -> passthrough to the
        // brainatlas endpoint.
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://brain.local:9000")
                .with_regions(vec![region("R1")])
                .with_post_responder(ok_two_queries()),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        runner.generate_queries_for_new_regions().await.expect("ok");

        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(
            calls[0].url,
            "http://brain.local:9000/brainatlas-be/api/generate-queries"
        );
    }

    #[tokio::test]
    async fn generate_queries_rewrites_0_0_0_0_to_localhost() {
        // No scheme + 0.0.0.0 -> normalized to http://localhost:PORT.
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "0.0.0.0:8082")
                .with_regions(vec![region("R1")])
                .with_post_responder(ok_two_queries()),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        runner.generate_queries_for_new_regions().await.expect("ok");

        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(
            calls[0].url,
            "http://localhost:8082/brainatlas-be/api/generate-queries"
        );
    }

    #[tokio::test]
    async fn generate_queries_strips_trailing_slash_from_base_url() {
        // Base URL from config has trailing slash; final URL must not have
        // a double slash before /brainatlas-be/...
        let infra = Arc::new(
            base_infra()
                .with_config(ConfigKey::BrainatlasBaseUrl, "http://brain.local:9000/")
                .with_regions(vec![region("R1")])
                .with_post_responder(ok_two_queries()),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        runner.generate_queries_for_new_regions().await.expect("ok");

        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(
            calls[0].url,
            "http://brain.local:9000/brainatlas-be/api/generate-queries"
        );
    }

    #[tokio::test]
    async fn generate_queries_errors_when_brainatlas_url_missing() {
        // Env var unset AND config unset -> ConfigNotFound surfaces.
        let infra = Arc::new(base_infra().with_regions(vec![region("R1")]));
        let runner = OrchPipelineRunner::new(infra);
        match runner.generate_queries_for_new_regions().await {
            Err(ServiceError::ConfigNotFound { key }) => {
                assert_eq!(key, "brainatlas_base_url");
            }
            other => panic!(
                "expected ConfigNotFound, got {:?}",
                other.as_ref().err().map(|e| e.to_string())
            ),
        }
    }

    #[tokio::test]
    async fn generate_queries_propagates_db_error_getting_regions() {
        // DB layer failing on the very first call surfaces as InfraError.
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_regions_without_queries_err("boom"),
        );
        let runner = OrchPipelineRunner::new(infra);
        match runner.generate_queries_for_new_regions().await {
            Err(ServiceError::InfraError(e)) => {
                assert!(e.to_string().contains("boom"), "got: {}", e);
            }
            other => panic!(
                "expected InfraError, got {:?}",
                other.as_ref().err().map(|e| e.to_string())
            ),
        }
    }

    #[tokio::test]
    async fn generate_queries_processes_all_regions_via_parallel_stream() {
        // Three regions -> three POSTs -> three insert_queries calls -> the
        // aggregate counters reflect every region.
        let r1 = region("R1");
        let r2 = region("R2");
        let r3 = region("R3");
        let ids = [r1.id, r2.id, r3.id];
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_config(ConfigKey::MaxParallelProcessCalls, "3")
                .with_regions(vec![r1, r2, r3])
                .with_post_responder(ok_two_queries()),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        let (regions_processed, total_queries) =
            runner.generate_queries_for_new_regions().await.expect("ok");

        assert_eq!(regions_processed, 3);
        assert_eq!(total_queries, 6); // 2 queries per region * 3 regions

        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(calls.len(), 3);

        let inserted = infra.inserted_queries.lock().unwrap();
        assert_eq!(inserted.len(), 3);
        let inserted_ids: std::collections::HashSet<_> =
            inserted.iter().map(|(id, _)| *id).collect();
        let expected_ids: std::collections::HashSet<_> = ids.into_iter().collect();
        assert_eq!(inserted_ids, expected_ids);
    }

    #[tokio::test]
    async fn generate_queries_swallows_per_region_failure() {
        // Region "BAD" always fails (all 3 retry attempts); other regions
        // succeed. The good regions still get their queries stored.
        let good = region("GOOD");
        let bad = region("BAD");
        let good_id = good.id;
        let bad_id = bad.id;
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_config(ConfigKey::MaxParallelProcessCalls, "2")
                .with_regions(vec![good, bad])
                .with_post_responder(Box::new(move |_idx, _url, body| {
                    let name = body["region_name"].as_str().unwrap_or("");
                    if name == "BAD" {
                        Err(MockErr("llm exploded".into()))
                    } else {
                        Ok(serde_json::json!({ "queries": ["q1", "q2"] }))
                    }
                })),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        let (regions_processed, total_queries) =
            runner.generate_queries_for_new_regions().await.expect("ok");

        assert_eq!(regions_processed, 1);
        assert_eq!(total_queries, 2);

        let inserted = infra.inserted_queries.lock().unwrap();
        assert_eq!(inserted.len(), 1);
        assert_eq!(inserted[0].0, good_id);
        assert_ne!(inserted[0].0, bad_id);
    }

    #[tokio::test]
    async fn generate_queries_empty_llm_response_is_swallowed() {
        // LLM returns an empty queries array -> no insert_queries call,
        // no increment of processed counter.
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_regions(vec![region("R1")])
                .with_post_responder(Box::new(|_idx, _url, _body| {
                    Ok(serde_json::json!({ "queries": [] }))
                })),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        let (regions_processed, total_queries) =
            runner.generate_queries_for_new_regions().await.expect("ok");

        assert_eq!(regions_processed, 0);
        assert_eq!(total_queries, 0);
        assert!(infra.inserted_queries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn generate_queries_retries_transient_failure_then_succeeds() {
        // backon::ExponentialBuilder with with_max_times(2) permits up to
        // 3 total attempts. Fail on the first call, succeed on the second.
        // Sleeps at least min_delay=1s between attempts, so this is a
        // ~1s test — still well within any sane CI budget.
        let attempts = Arc::new(Mutex::new(0usize));
        let attempts_clone = attempts.clone();
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_regions(vec![region("R1")])
                .with_post_responder(Box::new(move |_idx, _url, _body| {
                    let mut n = attempts_clone.lock().unwrap();
                    *n += 1;
                    if *n == 1 {
                        Err(MockErr("transient".into()))
                    } else {
                        Ok(serde_json::json!({ "queries": ["q1"] }))
                    }
                })),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        let (regions_processed, total_queries) =
            runner.generate_queries_for_new_regions().await.expect("ok");

        assert_eq!(regions_processed, 1);
        assert_eq!(total_queries, 1);
        // Two HTTP attempts were recorded — the first failed, the retry
        // succeeded.
        assert_eq!(*attempts.lock().unwrap(), 2);
        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
    }

    #[tokio::test]
    async fn generate_queries_circuit_breaker_trips_after_5_failures() {
        // Every region fails. After 5 consecutive failures, `circuit_broken`
        // (AtomicBool) flips and remaining regions return early. With 7
        // regions and concurrency=1, the last 2 must be short-circuited
        // and never hit the HTTP client.
        //
        // Note: backon retries 3 times per region (1 + 2 retries), so each
        // *failed region* burns 3 http attempts. We set up so only the
        // initial Err counts toward `consecutive_failures` (which is how
        // the code is written — cf. `pipeline_runner.rs:205`).
        let regions: Vec<RegionInfo> = (0..7).map(|i| region(&format!("R{}", i))).collect();
        let attempts = Arc::new(Mutex::new(0usize));
        let attempts_clone = attempts.clone();
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_config(ConfigKey::MaxParallelProcessCalls, "1")
                .with_regions(regions)
                .with_post_responder(Box::new(move |_idx, _url, _body| {
                    *attempts_clone.lock().unwrap() += 1;
                    Err(MockErr("always fails".into()))
                })),
        );
        let runner = OrchPipelineRunner::new(infra.clone());
        let (regions_processed, total_queries) =
            runner.generate_queries_for_new_regions().await.expect("ok");

        assert_eq!(regions_processed, 0);
        assert_eq!(total_queries, 0);

        // After 5 failures the circuit breaker trips. The remaining 2
        // regions skip the HTTP call entirely, so the recorded POST count
        // corresponds to the first 5 regions only.
        // Each of the 5 failed regions burns 3 retry attempts = 15 POSTs.
        let calls = infra.post_calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            5 * 3,
            "expected 5 regions * 3 retries = 15 POSTs before the breaker trips"
        );
    }

    // ======================================================================
    //  get_pending_fetch_task_count (lightweight accessor)
    // ======================================================================

    #[tokio::test]
    async fn get_pending_fetch_task_count_returns_db_value() {
        let infra = Arc::new(base_infra().with_pending_count(42));
        let runner = OrchPipelineRunner::new(infra);
        let n = runner.get_pending_fetch_task_count().await.expect("ok");
        assert_eq!(n, 42);
    }

    // ======================================================================
    //  generate_queries_for_new_regions_count
    // ======================================================================

    #[tokio::test]
    async fn generate_queries_for_new_regions_count_matches_region_list() {
        let infra =
            Arc::new(base_infra().with_regions(vec![region("A"), region("B"), region("C")]));
        let runner = OrchPipelineRunner::new(infra);
        let n = runner
            .generate_queries_for_new_regions_count()
            .await
            .expect("ok");
        assert_eq!(n, 3);
    }
}
