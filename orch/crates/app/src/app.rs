use crate::Services;
use domain::{
    BatchStatus, BatchStatusResult, ConfigEntry, ConfigEntryUpdate, ConfigKey,
    GenerateSummaryResult, InvalidateResult, PipelineStatsResult, Priority, RegionPipelineStatus,
    RegionStatusResult, SearchRegionResult,
};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub struct OrchApp<S> {
    services: Arc<S>,
}

impl<E, S> OrchApp<S>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }

    /// Initialize the orchestrator
    /// Spawns the background completion watcher loop and the pipeline runner loop
    pub async fn init(&self) -> Result<(), E> {
        let services = Arc::clone(&self.services);

        // Spawn the completion watcher loop (existing — promotes batches, triggers chunk+embed)
        let watcher_services = Arc::clone(&services);
        tokio::spawn(async move {
            loop {
                // Poll for completed tasks
                match watcher_services.poll().await {
                    Ok(poll_result) => {
                        tracing::info!(
                            total = poll_result.total_found,
                            ready = poll_result.tasks.len(),
                            already_processed = poll_result.already_processed,
                            "Poll completed"
                        );

                        // Process the tasks if any
                        if !poll_result.tasks.is_empty() {
                            match watcher_services.process(poll_result.tasks).await {
                                Ok(process_result) => {
                                    tracing::info!(
                                        success = process_result.successful,
                                        failed = process_result.failed,
                                        "Process completed"
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(error = ?e, "Failed to process tasks");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = ?e, "Failed to poll for tasks");
                    }
                }

                // Get the poll interval from config (default to 30 seconds)
                let interval_secs = match watcher_services
                    .get_config(ConfigKey::CompletionPollIntervalSecs)
                    .await
                {
                    Ok(Some(value)) => value.parse::<u64>().unwrap_or(30),
                    _ => 30,
                };

                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
            }
        });

        // Spawn the pipeline runner loop (new — generates queries, discovers papers, ensures workers)
        let pipeline_services = Arc::clone(&services);
        tokio::spawn(async move {
            // Small initial delay to let the server finish starting
            tokio::time::sleep(Duration::from_secs(10)).await;
            tracing::info!("Pipeline runner started");

            let mut cycle_count: u64 = 0;

            loop {
                tracing::info!(cycle = cycle_count, "Starting pipeline cycle");

                // === Phase 1: Generate queries for regions that don't have any ===
                match pipeline_services.generate_queries_for_new_regions().await {
                    Ok((regions, queries)) => {
                        if regions > 0 {
                            tracing::info!(
                                regions_processed = regions,
                                queries_generated = queries,
                                "Phase 1 complete: query generation"
                            );
                        }
                    }
                    Err(e) => tracing::error!(error = ?e, "Phase 1 failed: query generation"),
                }

                // === Phase 2: Re-run all queries to discover new papers ===
                match pipeline_services.discover_new_papers().await {
                    Ok((regions, tasks)) => {
                        if tasks > 0 {
                            tracing::info!(
                                regions_scanned = regions,
                                new_tasks = tasks,
                                "Phase 2 complete: paper discovery"
                            );
                        }
                    }
                    Err(e) => tracing::error!(error = ?e, "Phase 2 failed: paper discovery"),
                }

                // === Phase 3: Ensure fetcher workers are running (one-shot check) ===
                if let Err(e) = pipeline_services.ensure_fetcher_running().await {
                    tracing::warn!(error = ?e, "Phase 3: Failed to ensure workers running");
                }

                cycle_count += 1;

                // Sleep between cycles
                let sleep_secs = match pipeline_services
                    .get_config(ConfigKey::PipelineCycleSleepSecs)
                    .await
                {
                    Ok(Some(v)) => v.parse::<u64>().unwrap_or(3600),
                    _ => 3600,
                };
                tracing::info!(
                    cycle = cycle_count,
                    sleep_secs,
                    "Pipeline cycle complete, sleeping"
                );
                tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
            }
        });

        // Spawn the fast-cadence fetcher monitor loop (independent of the slow pipeline cycle).
        // While the fetch queue is non-empty, this loop re-probes worker health every
        // `fetcher_monitor_interval_secs` (default 30s) and re-allocates dead workers.
        // When the queue drains, it sleeps at the same cadence without probing workers.
        let monitor_services = Arc::clone(&services);
        tokio::spawn(async move {
            // Let the pipeline runner get a head start
            tokio::time::sleep(Duration::from_secs(15)).await;
            tracing::info!("Fetcher monitor loop started");

            loop {
                // Read fast-loop interval from config (default 30s)
                let interval_secs = match monitor_services
                    .get_config(ConfigKey::FetcherMonitorIntervalSecs)
                    .await
                {
                    Ok(Some(v)) => v.parse::<u64>().unwrap_or(30),
                    _ => 30,
                };

                // Check pending task count — only probe workers when there's work to do
                match monitor_services.get_pending_fetch_task_count().await {
                    Ok(pending) if pending > 0 => {
                        tracing::debug!(
                            pending_tasks = pending,
                            "Fetcher monitor: queue non-empty, checking workers"
                        );

                        if let Err(e) = monitor_services.ensure_fetcher_running().await {
                            tracing::warn!(
                                error = ?e,
                                "Fetcher monitor: failed to ensure workers running"
                            );
                        }
                    }
                    Ok(pending) => {
                        tracing::debug!(
                            pending_tasks = pending,
                            "Fetcher monitor: queue empty, sleeping"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = ?e,
                            "Fetcher monitor: failed to get pending task count"
                        );
                    }
                }

                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
            }
        });

        // Spawn the Phase-4 eval orchestrator loop.
        // Gated on `ConfigKey::EvalOrchestratorEnabled` (config-table driven, hot-reloadable).
        // Each cycle: ask evals-be for unscored summary IDs, fan out
        // `POST /evals-be/api/evals/score` calls at configured concurrency.
        let eval_services = Arc::clone(&services);
        tokio::spawn(async move {
            // Generous initial delay so this loop never races startup of the
            // other services.
            tokio::time::sleep(Duration::from_secs(20)).await;
            tracing::info!("Eval orchestrator loop started");

            loop {
                let interval_secs = eval_services.eval_orchestrator_poll_interval_secs().await;

                if eval_services.eval_orchestrator_enabled().await {
                    match eval_services.eval_orchestrator_run_cycle().await {
                        Ok((succeeded, failed)) if succeeded + failed > 0 => {
                            tracing::info!(succeeded, failed, "Eval orchestrator: cycle complete");
                        }
                        Ok(_) => {
                            tracing::debug!("Eval orchestrator: no unscored summaries");
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "Eval orchestrator: cycle failed");
                        }
                    }
                } else {
                    tracing::debug!("Eval orchestrator: disabled, skipping cycle");
                }

                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
            }
        });

        // ── Cost guardrail ───────────────────────────────────────────────
        // Background loop that polls brainatlas-be's `/api/llm/usage` for
        // the rolling 24h LLM spend and emits `tracing::warn!`/`error!`
        // events when configured thresholds are breached. Never blocks any
        // call — observability only. Gated on `ConfigKey::CostGuardrailEnabled`.
        let cost_services = Arc::clone(&services);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(25)).await;
            tracing::info!("Cost guardrail loop started");

            loop {
                let interval_secs = cost_services.cost_guardrail_poll_interval_secs().await;

                if cost_services.cost_guardrail_enabled().await {
                    // Run once; errors are swallowed inside so this never
                    // propagates.
                    let _ = cost_services.cost_guardrail_run_once().await;
                } else {
                    tracing::debug!("Cost guardrail: disabled, skipping cycle");
                }

                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
            }
        });

        Ok(())
    }

    /// Search for summaries of a brain region
    /// If summaries exist, return them
    /// If not, create a batch and return status
    /// Search for a region (deprecated - use list_summaries + generate_summary + get_batch_status)
    pub async fn search_region(&self, region_id: Uuid) -> Result<SearchRegionResult, E> {
        tracing::warn!("search_region is deprecated, use list_summaries instead");
        self.list_summaries(region_id).await
    }

    /// Get the status of processing for a region
    pub async fn get_region_status(&self, region_id: Uuid) -> Result<RegionStatusResult, E> {
        // Get active batch if exists
        let batch = self.services.get_active_batch(region_id).await?;

        // Get summaries count
        let summaries = self.services.get_summaries(region_id).await?;

        let status = if !summaries.is_empty() {
            RegionPipelineStatus::Done
        } else if let Some(batch) = batch.as_ref() {
            match batch.status {
                domain::BatchStatus::Collecting => RegionPipelineStatus::FetchQueued,
                domain::BatchStatus::Ready => RegionPipelineStatus::LlmQueued,
                domain::BatchStatus::Processing => RegionPipelineStatus::Processing,
                domain::BatchStatus::Completed => RegionPipelineStatus::Done,
                domain::BatchStatus::Failed => RegionPipelineStatus::FetchFailed,
                domain::BatchStatus::Invalidated => RegionPipelineStatus::Invalidated,
            }
        } else {
            RegionPipelineStatus::NotStarted
        };

        Ok(RegionStatusResult {
            region_id,
            status,
            last_fetch_at: batch.as_ref().and_then(|b| b.completed_at),
            last_summary_at: summaries.first().map(|s| s.created_at),
            summary_count: summaries.len() as i32,
            current_priority: batch.as_ref().and({
                // Priority would come from the batch's tasks
                // For now, return None - could be enhanced to lookup from fetch_tasks
                None
            }),
        })
    }

    /// Invalidate existing summaries and re-process
    pub async fn invalidate_region(
        &self,
        region_id: Uuid,
        _priority: Option<Priority>,
    ) -> Result<InvalidateResult, E> {
        tracing::info!(?region_id, "Invalidating region");

        // Step 1: Delete all queries for this region to force regeneration
        self.services.delete_queries(region_id).await?;
        tracing::info!(?region_id, "Deleted existing queries");

        // Step 2: Get active batch if exists
        // Step 2: Get the most recent batch (including completed ones)
        let recent_batch = self.services.get_recent_batch(region_id).await?;

        let detail = if let Some(batch) = &recent_batch {
            // Mark batch as invalidated (even if it was completed)
            self.services
                .update_batch_status(
                    batch.id,
                    domain::BatchStatus::Invalidated,
                    Some("Invalidated by user".to_string()),
                )
                .await?;
            tracing::info!(?region_id, batch_id=?batch.id, "Marked batch as invalidated");

            format!(
                "Batch {} marked as invalidated. Queries deleted. Next search will create a new batch with fresh queries.",
                batch.id
            )
        } else {
            "No batch found. Queries deleted. A new batch with fresh queries will be created on next search.".to_string()
        };
        tracing::info!(?region_id, "Region invalidated successfully");

        Ok(InvalidateResult {
            region_id,
            new_status: RegionPipelineStatus::Invalidated,
            detail,
        })
    }

    /// Get pipeline statistics across all regions
    pub async fn get_pipeline_stats(&self) -> Result<PipelineStatsResult, E> {
        use std::collections::HashMap;
        use std::collections::hash_map::Entry;

        // Get total region count and regions that have never been touched
        let total_regions = self.services.get_total_regions().await? as i32;
        let not_started = self.services.count_regions_without_batches().await? as i32;

        // Collect all batches across every status, then keep only the *latest*
        // batch per region so we count regions rather than batch records.
        // (A region that failed, was invalidated, and restarted would otherwise
        // be counted multiple times.)
        let mut latest_by_region: HashMap<Uuid, domain::ProcessingBatch> = HashMap::new();

        for status in [
            BatchStatus::Collecting,
            BatchStatus::Ready,
            BatchStatus::Processing,
            BatchStatus::Completed,
            BatchStatus::Failed,
            BatchStatus::Invalidated,
        ] {
            for batch in self.services.get_batches_by_status(status).await? {
                match latest_by_region.entry(batch.region_id) {
                    Entry::Vacant(e) => {
                        e.insert(batch);
                    }
                    Entry::Occupied(mut e) => {
                        if batch.created_at > e.get().created_at {
                            e.insert(batch);
                        }
                    }
                }
            }
        }

        let (
            mut fetch_queued,
            mut fetch_failed,
            mut llm_queued,
            mut processing,
            mut done,
            mut invalidated,
        ) = (0i32, 0i32, 0i32, 0i32, 0i32, 0i32);

        for batch in latest_by_region.values() {
            match batch.status {
                BatchStatus::Collecting => fetch_queued += 1,
                BatchStatus::Ready => llm_queued += 1,
                BatchStatus::Processing => processing += 1,
                BatchStatus::Completed => done += 1,
                BatchStatus::Failed => fetch_failed += 1,
                BatchStatus::Invalidated => invalidated += 1,
            }
        }

        // Count collecting batches that have at least one in_progress fetch task
        let fetching = self.services.count_actively_fetching_regions().await? as i32;

        Ok(PipelineStatsResult {
            total_regions,
            not_started,
            fetch_queued,
            fetching,
            fetch_failed,
            llm_queued,
            processing,
            done,
            invalidated,
        })
    }

    /// Get all configuration entries
    pub async fn get_config(&self) -> Result<Vec<ConfigEntry>, E> {
        self.services.get_all_config().await
    }

    /// Update configuration entries
    pub async fn update_config(
        &self,
        entries: Vec<ConfigEntryUpdate>,
    ) -> Result<Vec<ConfigEntry>, E> {
        self.services.update_config(entries).await
    }

    /// Aggregate eval status (proxies to evals-be `/api/evals/summary`).
    pub async fn get_eval_status(&self) -> Result<crate::EvalStatusSummary, E> {
        self.services.eval_orchestrator_get_status().await
    }

    /// `N` lowest-scoring summaries for one metric (proxies to evals-be
    /// `/api/evals/worst`).
    pub async fn get_eval_worst(
        &self,
        metric: String,
        limit: i64,
    ) -> Result<crate::EvalWorstOffenders, E> {
        self.services
            .eval_orchestrator_get_worst(metric, limit)
            .await
    }

    /// Aggregate LLM cost for one eval run. Proxies to brainatlas-be's
    /// `/api/llm/usage?correlation_id_prefix=eval:{run_id}:`.
    pub async fn get_eval_run_cost(&self, run_id: uuid::Uuid) -> Result<domain::EvalRunCost, E> {
        self.services.eval_orchestrator_get_run_cost(run_id).await
    }

    /// Get all brain regions from region_mapping
    pub async fn get_all_regions(&self) -> Result<Vec<domain::Region>, E> {
        self.services.get_all_regions().await
    }

    /// List all summaries for a region (no status logic, just list)
    pub async fn list_summaries(&self, region_id: Uuid) -> Result<SearchRegionResult, E> {
        tracing::info!(?region_id, "Listing summaries for region");

        let summaries = self.services.get_summaries(region_id).await?;

        Ok(SearchRegionResult { summaries })
    }

    /// Generate a new summary for a region using the existing knowledge base.
    /// If queries exist (from pipeline), uses them. Otherwise generates new ones.
    /// If there's already an active batch, returns that batch's info instead.
    pub async fn generate_summary(&self, region_id: Uuid) -> Result<GenerateSummaryResult, E> {
        tracing::info!(?region_id, "Generating new summary for region");

        // Step 1: Check if there's already an active batch for this region
        if let Some(active_batch) = self.services.get_active_batch(region_id).await? {
            tracing::warn!(
                ?region_id,
                batch_id = ?active_batch.id,
                status = ?active_batch.status,
                "Batch already in progress, returning existing batch info"
            );
            return Ok(GenerateSummaryResult {
                batch_id: active_batch.id,
                query_count: 0, // We don't track this for existing batches
                task_count: active_batch.fetch_task_ids.len(),
                already_in_progress: true,
            });
        }

        // Step 2: Check for existing queries (pre-generated by pipeline)
        let existing_queries = self.services.get_queries(region_id).await?;
        let queries = if !existing_queries.is_empty() {
            tracing::info!(
                ?region_id,
                count = existing_queries.len(),
                "Using pre-generated queries from pipeline"
            );
            existing_queries
                .into_iter()
                .map(|q| q.query_text)
                .collect::<Vec<_>>()
        } else {
            // No pre-generated queries — generate them now
            let region_name = self.services.get_region_name(region_id).await?;
            let query_count = self
                .services
                .get_query_generation_limit()
                .await?
                .unwrap_or(3);

            let generated = self
                .services
                .generate_queries(&region_name, query_count)
                .await?;

            if !generated.is_empty() {
                // Store them for future use
                self.services
                    .store_queries(region_id, generated.clone())
                    .await?;
            }

            generated
        };

        if queries.is_empty() {
            tracing::warn!(?region_id, "No queries available");
            return Ok(GenerateSummaryResult {
                batch_id: Uuid::nil(),
                query_count: 0,
                task_count: 0,
                already_in_progress: false,
            });
        }

        tracing::info!(?region_id, query_count = queries.len(), "Using queries");

        // Step 3: Enqueue fetch tasks for each query.
        //
        // The enqueue call discovers papers via NCBI and returns task IDs.
        // With UNIQUE(pmc_id) + DO NOTHING, existing papers return their
        // existing task IDs without creating duplicates.
        //
        // We do NOT use `?` inside this loop. A single failed enqueue must not
        // abort the entire loop and leave the batch with an empty fetch_task_ids
        // array — that would cause a zombie batch stuck in `collecting` forever.
        let mut all_task_ids = Vec::new();
        for query in &queries {
            match self
                .services
                .enqueue_fetch_task(
                    query.clone(),
                    region_id,
                    domain::Priority::UserRequested.as_i32(),
                )
                .await
            {
                Ok(query_task_ids) => all_task_ids.extend(query_task_ids),
                Err(e) => {
                    tracing::warn!(?region_id, query, error = %e, "Failed to enqueue fetch task for query, skipping")
                }
            }
        }

        if all_task_ids.is_empty() {
            // No papers found at all — create a failed batch
            let batch_id = self.services.create_batch(region_id, 0).await?;
            tracing::warn!(
                ?region_id,
                ?batch_id,
                "No papers found, marking batch as failed"
            );
            self.services
                .update_batch_status(
                    batch_id,
                    domain::BatchStatus::Failed,
                    Some("No papers found".to_string()),
                )
                .await?;

            return Ok(GenerateSummaryResult {
                batch_id,
                query_count: queries.len(),
                task_count: 0,
                already_in_progress: false,
            });
        }

        // Deduplicate: ON CONFLICT(pmc_id) DO NOTHING returns the same task ID
        // when the same paper is found by multiple queries. Without this, the
        // batch's fetch_task_ids array contains duplicates and the completion
        // watcher's `completed_count == array.len()` check never matches.
        let all_task_ids: Vec<i64> = {
            let mut seen = std::collections::HashSet::new();
            all_task_ids
                .into_iter()
                .filter(|id| seen.insert(*id))
                .collect()
        };

        // Step 4: Filter to only already-completed tasks.
        //
        // The knowledge base is continuously expanding via the background
        // pipeline. We use whatever papers are ALREADY fetched rather than
        // blocking the user while pending papers download. If zero papers
        // are completed, we fall back to the full set (collecting mode).
        let completed_task_ids = self
            .services
            .get_completed_task_ids(all_task_ids.clone())
            .await?;

        let (batch_task_ids, start_as_ready) = if !completed_task_ids.is_empty() {
            tracing::info!(
                ?region_id,
                total_discovered = all_task_ids.len(),
                already_completed = completed_task_ids.len(),
                "Using already-fetched papers for immediate processing"
            );
            (completed_task_ids, true)
        } else {
            tracing::info!(
                ?region_id,
                total_discovered = all_task_ids.len(),
                "No papers fetched yet — batch will wait in collecting"
            );
            (all_task_ids, false)
        };

        let task_count = batch_task_ids.len();

        // Step 5: Create batch and add tasks
        let batch_id = self.services.create_batch(region_id, task_count).await?;
        tracing::info!(
            ?region_id,
            ?batch_id,
            task_count,
            start_as_ready,
            "Created batch"
        );

        self.services
            .add_tasks_to_batch(batch_id, batch_task_ids)
            .await?;

        // Step 6: If all tasks are already completed, promote directly to Ready
        // so the completion watcher processes it on the next cycle (~30s).
        if start_as_ready {
            self.services
                .update_batch_status(batch_id, domain::BatchStatus::Ready, None)
                .await?;
            tracing::info!(
                ?region_id,
                ?batch_id,
                "Batch promoted to Ready — will be processed on next watcher cycle"
            );
        }

        // Step 7: Ensure workers are allocated (for any remaining pending tasks)
        if let Err(e) = self.services.ensure_workers_allocated().await {
            tracing::warn!(
                ?e,
                "Failed to ensure workers allocated, tasks will remain queued"
            );
        }

        Ok(GenerateSummaryResult {
            batch_id,
            query_count: queries.len(),
            task_count,
            already_in_progress: false,
        })
    }

    /// Get the status of a batch
    pub async fn get_batch_status(&self, batch_id: Uuid) -> Result<BatchStatusResult, E> {
        tracing::info!(?batch_id, "Getting batch status");

        // Get batch details from database
        let batch = self.services.get_batch_by_id(batch_id).await?;
        let batch = batch.expect("Batch not found");

        // Map batch status to pipeline status
        let status = match batch.status {
            domain::BatchStatus::Collecting => domain::RegionPipelineStatus::Fetching,
            domain::BatchStatus::Ready => domain::RegionPipelineStatus::LlmQueued,
            domain::BatchStatus::Processing => domain::RegionPipelineStatus::Processing,
            domain::BatchStatus::Completed => domain::RegionPipelineStatus::Done,
            domain::BatchStatus::Failed => domain::RegionPipelineStatus::FetchFailed,
            domain::BatchStatus::Invalidated => domain::RegionPipelineStatus::Invalidated,
        };

        // Generate appropriate message
        let message = match batch.status {
            domain::BatchStatus::Collecting => format!(
                "Fetching papers from PubMed Central ({} tasks)",
                batch.expected_task_count
            ),
            domain::BatchStatus::Ready => "Papers fetched, waiting for LLM processing".to_string(),
            domain::BatchStatus::Processing => "Generating summary with LLM".to_string(),
            domain::BatchStatus::Completed => "Summary generation complete".to_string(),
            domain::BatchStatus::Failed => format!(
                "Failed: {}",
                batch.error_message.as_deref().unwrap_or("Unknown error")
            ),
            domain::BatchStatus::Invalidated => "Batch was invalidated".to_string(),
        };

        // Count completed tasks for this batch
        let completed_tasks = if !batch.fetch_task_ids.is_empty() {
            self.services
                .count_completed_tasks(batch.fetch_task_ids.clone())
                .await
                .ok()
        } else {
            None
        };

        Ok(BatchStatusResult {
            batch_id,
            status,
            message,
            error: batch.error_message,
            expected_tasks: batch.expected_task_count,
            completed_tasks,
            created_at: batch.created_at,
        })
    }

    /// Get the active batch ID for a region (if one exists)
    /// Returns None if no active batch is in progress
    pub async fn get_active_batch_id(&self, region_id: Uuid) -> Result<Option<Uuid>, E> {
        tracing::info!(?region_id, "Getting active batch ID for region");

        let active_batch = self.services.get_active_batch(region_id).await?;
        Ok(active_batch.map(|batch| batch.id))
    }

    /// Reverse search: find brain regions by natural language query
    pub async fn reverse_search(&self, query: &str) -> Result<domain::SearchResponse, E> {
        tracing::info!(query, "Performing reverse search");
        self.services.reverse_search(query).await
    }

    /// Lightweight pipeline health snapshot: region/query/task counts + active workers.
    /// Calls existing infra queries + worker status — no shared state needed.
    pub async fn get_pipeline_status(&self) -> Result<domain::PipelineHealthStatus, E> {
        // Fire all queries concurrently for speed
        let (pending_result, workers_result, regions_without_result, regions_with_result) = tokio::join!(
            self.services.get_pending_fetch_task_count(),
            self.services.get_worker_status(),
            self.services.generate_queries_for_new_regions_count(),
            self.services.get_regions_with_queries_count(),
        );

        let pending_fetch_tasks = pending_result.unwrap_or(0);
        let worker_count = workers_result
            .map(|ws| ws.iter().filter(|w| w.status == "running").count())
            .unwrap_or(0);
        let regions_without_queries = regions_without_result.unwrap_or(0) as usize;
        let regions_with_queries = regions_with_result.unwrap_or(0) as usize;

        Ok(domain::PipelineHealthStatus {
            regions_without_queries,
            regions_with_queries,
            pending_fetch_tasks,
            worker_count,
        })
    }

    /// Get comprehensive system stats for the dev dashboard
    pub async fn get_system_stats(&self) -> Result<domain::SystemStats, E> {
        self.services.get_system_stats().await
    }

    /// Per-region summary freshness (fresh / stale / no_summary), bucketed by
    /// the `summary_staleness_days` config value.
    pub async fn get_summary_freshness(&self) -> Result<domain::SummaryFreshness, E> {
        self.services.get_summary_freshness().await
    }

    /// Snapshot of the Redis cache (key counts per prefix, memory, hit rate).
    /// Errors are surfaced inside the response body, not propagated.
    pub async fn get_redis_stats(&self) -> Result<domain::RedisStats, E> {
        self.services.get_redis_stats().await
    }

    /// Manually trigger pipeline phases on demand. Phases run sequentially and
    /// independently: a failure in one phase is collected into `errors` but
    /// does NOT abort subsequent phases (the operator can retry piecemeal).
    pub async fn trigger_pipeline(
        &self,
        req: domain::PipelineTriggerRequest,
    ) -> Result<domain::PipelineTriggerResult, E> {
        let mut result = domain::PipelineTriggerResult::default();

        if req.reset_queries {
            tracing::warn!("trigger_pipeline: wiping all region_queries rows");
            match self.services.delete_all_queries().await {
                Ok(deleted) => {
                    result.reset_queries_deleted = Some(deleted);
                }
                Err(e) => {
                    result.errors.push(format!("reset_queries: {}", e));
                }
            }
        }

        if req.generate_queries {
            tracing::info!("trigger_pipeline: Phase 1 (generate_queries)");
            match self.services.generate_queries_for_new_regions().await {
                Ok((rp, tq)) => {
                    result.generate_queries_result = Some((rp, tq));
                }
                Err(e) => {
                    result.errors.push(format!("generate_queries: {}", e));
                }
            }
        }

        if req.discover_papers {
            tracing::info!("trigger_pipeline: Phase 2 (discover_papers)");
            match self.services.discover_new_papers().await {
                Ok((rs, nt)) => {
                    result.discover_papers_result = Some((rs, nt));
                }
                Err(e) => {
                    result.errors.push(format!("discover_papers: {}", e));
                }
            }
        }

        if req.ensure_workers {
            tracing::info!("trigger_pipeline: Phase 3 (ensure_workers)");
            match self.services.ensure_fetcher_running().await {
                Ok(()) => {
                    result.ensure_workers_ok = Some(true);
                }
                Err(e) => {
                    result.ensure_workers_ok = Some(false);
                    result.errors.push(format!("ensure_workers: {}", e));
                }
            }
        }

        Ok(result)
    }
}
