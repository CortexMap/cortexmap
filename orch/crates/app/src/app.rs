use crate::Services;
use domain::{BatchStatus, BatchStatusResult, ConfigEntry, ConfigEntryUpdate, ConfigKey, GenerateSummaryResult, InvalidateResult, PipelineStatsResult, Priority, RegionPipelineStatus, RegionStatusResult, SearchRegionResult};
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
    /// Spawns the background completion watcher loop
    pub async fn init(&self) -> Result<(), E> {
        let services = Arc::clone(&self.services);

        tokio::spawn(async move {
            loop {
                // Poll for completed tasks
                match services.poll().await {
                    Ok(poll_result) => {
                        tracing::info!(
                            total = poll_result.total_found,
                            ready = poll_result.tasks.len(),
                            already_processed = poll_result.already_processed,
                            "Poll completed"
                        );

                        // Process the tasks if any
                        if !poll_result.tasks.is_empty() {
                            match services.process(poll_result.tasks).await {
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
                let interval_secs = match services
                    .get_config(ConfigKey::CompletionPollIntervalSecs)
                    .await
                {
                    Ok(Some(value)) => value.parse::<u64>().unwrap_or(30),
                    _ => 30,
                };

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
    pub async fn invalidate_region(&self, region_id: Uuid, _priority: Option<Priority>) -> Result<InvalidateResult, E> {
        tracing::info!(?region_id, "Invalidating region");
        
        // Step 1: Delete all queries for this region to force regeneration
        self.services.delete_queries(region_id).await?;
        tracing::info!(?region_id, "Deleted existing queries");
        
        // Step 2: Get active batch if exists
        // Step 2: Get the most recent batch (including completed ones)
        let recent_batch = self.services.get_recent_batch(region_id).await?;

        let detail = if let Some(batch) = &recent_batch {
            // Mark batch as invalidated (even if it was completed)
            self.services.update_batch_status(
                batch.id,
                domain::BatchStatus::Invalidated,
                Some("Invalidated by user".to_string())
            ).await?;
            tracing::info!(?region_id, batch_id=?batch.id, "Marked batch as invalidated");

            format!("Batch {} marked as invalidated. Queries deleted. Next search will create a new batch with fresh queries.", batch.id)
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
        use std::collections::hash_map::Entry;
        use std::collections::HashMap;

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
                    Entry::Vacant(e) => { e.insert(batch); }
                    Entry::Occupied(mut e) => {
                        if batch.created_at > e.get().created_at {
                            e.insert(batch);
                        }
                    }
                }
            }
        }

        let (mut fetch_queued, mut fetch_failed, mut llm_queued, mut processing, mut done, mut invalidated) =
            (0i32, 0i32, 0i32, 0i32, 0i32, 0i32);

        for batch in latest_by_region.values() {
            match batch.status {
                BatchStatus::Collecting  => fetch_queued += 1,
                BatchStatus::Ready       => llm_queued   += 1,
                BatchStatus::Processing  => processing   += 1,
                BatchStatus::Completed   => done         += 1,
                BatchStatus::Failed      => fetch_failed += 1,
                BatchStatus::Invalidated => invalidated  += 1,
            }
        }

        Ok(PipelineStatsResult {
            total_regions,
            not_started,
            fetch_queued,
            fetching: 0, // No distinct "actively fetching" state; subsumed by fetch_queued
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
    pub async fn update_config(&self, entries: Vec<ConfigEntryUpdate>) -> Result<Vec<ConfigEntry>, E> {
        self.services.update_config(entries).await
    }
    
    /// Get all brain regions from region_mapping
    pub async fn get_all_regions(&self) -> Result<Vec<domain::Region>, E> {
        self.services.get_all_regions().await
    }

    /// List all summaries for a region (no status logic, just list)
    pub async fn list_summaries(&self, region_id: Uuid) -> Result<SearchRegionResult, E> {
        tracing::info!(?region_id, "Listing summaries for region");
        
        let summaries = self.services.get_summaries(region_id).await?;
        
        Ok(SearchRegionResult {
            summaries,
        })
    }

    /// Generate a new summary for a region (creates batch, enqueues tasks)
    /// If there's already an active batch, returns that batch's info instead
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
        
        // Step 2: Get region name
        let region_name = self.services.get_region_name(region_id).await?;
        
        // Step 3: Get query count from config (default to 3)
        let query_count = self.services.get_query_generation_limit().await?
            .unwrap_or(3);
        
        // Step 3: Generate queries via LLM
        let queries = self.services.generate_queries(&region_name, query_count).await?;
        
        if queries.is_empty() {
            tracing::warn!(?region_id, "No queries generated by LLM");
            // Just return empty result instead of error
            return Ok(GenerateSummaryResult {
                batch_id: Uuid::nil(),
                query_count: 0,
                task_count: 0,
                already_in_progress: false,
            });
        }
        
        tracing::info!(?region_id, query_count = queries.len(), "Generated queries");
        
        // Step 4: Create a new batch
        let batch_id = self.services.create_batch(region_id, queries.len()).await?;
        tracing::info!(?region_id, ?batch_id, "Created batch");
        
        // Step 5: Save queries
        self.services.store_queries(region_id, queries.clone()).await?;
        
        // Step 6: Enqueue fetch tasks for each query.
        //
        // We do NOT use `?` inside this loop. A single failed enqueue must not
        // abort the entire loop and leave the batch with an empty fetch_task_ids
        // array — that would cause a zombie batch stuck in `collecting` forever.
        // Instead we log the error and continue with whatever task IDs we did
        // collect, then call add_tasks_to_batch / mark failed below.
        let mut task_ids = Vec::new();
        for query in &queries {
            match self.services.enqueue_fetch_task(query.clone(), region_id, domain::Priority::UserRequested.as_i32()).await {
                Ok(query_task_ids) => task_ids.extend(query_task_ids),
                Err(e) => tracing::warn!(?region_id, ?batch_id, query, error = %e, "Failed to enqueue fetch task for query, skipping"),
            }
        }
        
        let task_count = task_ids.len();
        tracing::info!(?region_id, ?batch_id, task_count, "Enqueued fetch tasks");
        
        // Step 7: Add tasks to batch (if any)
        if !task_ids.is_empty() {
            self.services.add_tasks_to_batch(batch_id, task_ids).await?;
            
            // Step 8: Update expected count if mismatch
            if task_count != queries.len() {
                tracing::info!(?region_id, ?batch_id, expected = queries.len(), actual = task_count, "Updating batch expected count");
                self.services.update_batch_expected_count(batch_id, task_count as i32).await?;
            }
        } else {
            // No tasks created - mark batch as failed
            tracing::warn!(?region_id, ?batch_id, "No tasks created, marking batch as failed");
            self.services.update_batch_status(batch_id, domain::BatchStatus::Failed, Some("No papers found".to_string())).await?;
        }
        
        // Step 9: Ensure workers are allocated
        if let Err(e) = self.services.ensure_workers_allocated().await {
            tracing::warn!(?e, "Failed to ensure workers allocated, tasks will remain queued");
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
            domain::BatchStatus::Collecting => format!("Fetching papers from PubMed Central ({} tasks)", batch.expected_task_count),
            domain::BatchStatus::Ready => "Papers fetched, waiting for LLM processing".to_string(),
            domain::BatchStatus::Processing => "Generating summary with LLM".to_string(),
            domain::BatchStatus::Completed => "Summary generation complete".to_string(),
            domain::BatchStatus::Failed => format!("Failed: {}", batch.error_message.as_deref().unwrap_or("Unknown error")),
            domain::BatchStatus::Invalidated => "Batch was invalidated".to_string(),
        };
        
        // Count completed tasks for this batch
        let completed_tasks = if !batch.fetch_task_ids.is_empty() {
            match self.services.count_completed_tasks(batch.fetch_task_ids.clone()).await {
                Ok(count) => Some(count),
                Err(_) => None,
            }
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
}
