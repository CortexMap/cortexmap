use crate::Services;
use domain::{BatchStatus, ConfigEntry, ConfigEntryUpdate, ConfigKey, InvalidateResult, PipelineStatsResult, Priority, RegionPipelineStatus, RegionStatusResult, SearchRegionResult};
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
    pub async fn search_region(&self, region_id: Uuid) -> Result<SearchRegionResult, E> {
        tracing::info!(?region_id, "Searching for region");
        
        // STEP 1: Check for existing summaries
        let summaries = self.services.get_summaries(region_id).await
            .map_err(|e| e.into())?;
        
        if !summaries.is_empty() {
            tracing::info!(?region_id, count = summaries.len(), "Found existing summaries");
            return Ok(SearchRegionResult {
                status: RegionPipelineStatus::Done,
                summaries,
            });
        }
        
        tracing::info!(?region_id, "No summaries found, checking for batch");
        
        // STEP 3: Check for active batch
        let active_batch = self.services.get_active_batch(region_id).await
            .map_err(|e| e.into())?;
        
        if let Some(batch) = active_batch {
            tracing::info!(?region_id, ?batch.status, "Found active batch");
            
            // Derive status from batch state
            let status = match batch.status {
                domain::BatchStatus::Collecting => RegionPipelineStatus::Fetching,
                domain::BatchStatus::Ready => RegionPipelineStatus::LlmQueued,
                domain::BatchStatus::Processing => RegionPipelineStatus::Processing,
                domain::BatchStatus::Completed => RegionPipelineStatus::Done,
                domain::BatchStatus::Failed => RegionPipelineStatus::FetchFailed,
            };
            
            return Ok(SearchRegionResult {
                status,
                summaries: vec![],
            });
        }
        
        tracing::info!(?region_id, "No batch found, creating new batch");
        
        // STEP 4: No batch exists, create one
        
        // 4a. Check if queries exist for this region
        let queries = self.services.get_queries(region_id).await
            .map_err(|e| e.into())?;
        
        let query_strings: Vec<String> = if queries.is_empty() {
            tracing::info!(?region_id, "No queries found, generating new ones");
            
            // Get region name from region_mapping
            let region_name = self.services.get_region_name(region_id).await
                .map_err(|e| e.into())?;
            
            // Get query count from config (default to 3)
            let count = self.services.get_query_generation_limit().await
                .map_err(|e| e.into())?
                .unwrap_or(3);
            
            let generated = self.services.generate_queries(&region_name, count).await
                .map_err(|e| e.into())?;
            
            // Store generated queries
            self.services.store_queries(region_id, generated.clone()).await
                .map_err(|e| e.into())?;
            
            tracing::info!(?region_id, count = generated.len(), "Generated and stored queries");
            generated
        } else {
            tracing::info!(?region_id, count = queries.len(), "Using existing queries");
            queries.into_iter().map(|q| q.query_text).collect()
        };
        
        // 4b. Create batch
        let batch_id = self.services.create_batch(region_id, query_strings.len()).await
            .map_err(|e| e.into())?;
        
        tracing::info!(?region_id, ?batch_id, "Created batch");
        
        // 4c. Enqueue fetch tasks for each query
        let mut task_ids = Vec::new();
        for query in &query_strings {
            let task_id = self.services.enqueue_fetch_task(
                query.clone(),
                region_id,
                5, // Normal priority
            ).await.map_err(|e| e.into())?;
            
            task_ids.push(task_id);
        }
        
        tracing::info!(?region_id, ?batch_id, task_count = task_ids.len(), "Enqueued fetch tasks");
        
        // 4d. Link tasks to batch
        self.services.add_tasks_to_batch(batch_id, task_ids).await
            .map_err(|e| e.into())?;
        
        Ok(SearchRegionResult {
            status: RegionPipelineStatus::FetchQueued,
            summaries: vec![],
        })
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
            current_priority: batch.as_ref().and_then(|_b| {
                // Priority would come from the batch's tasks
                // For now, return None - could be enhanced to lookup from fetch_tasks
                None
            }),
        })
    }

    /// Invalidate existing summaries and re-process
    pub async fn invalidate_region(&self, region_id: Uuid, _priority: Option<Priority>) -> Result<InvalidateResult, E> {
        // Get active batch if exists
        let active_batch = self.services.get_active_batch(region_id).await?;
        
        let (detail, batch_existed) = if let Some(batch) = &active_batch {
            // Batch exists - reset it to collecting to trigger reprocess
            self.services.update_batch_status(
                batch.id,
                domain::BatchStatus::Collecting,
                None
            ).await?;
            
            (format!("Batch {} reset to collecting status for reprocessing", batch.id), true)
        } else {
            // No batch exists - invalidation will happen when user searches
            ("No active batch found. A new batch will be created on next search.".to_string(), false)
        };
        
        // Determine new status
        let new_status = if batch_existed {
            RegionPipelineStatus::Invalidated
        } else {
            RegionPipelineStatus::NotStarted
        };
        
        Ok(InvalidateResult {
            region_id,
            new_status,
            detail,
        })
    }

    /// Get pipeline statistics across all regions
    pub async fn get_pipeline_stats(&self) -> Result<PipelineStatsResult, E> {
        // Get total region count
        let total_regions = self.services.get_total_regions().await
            .map_err(|e| e.into())? as i32;
        
        // Get count of regions with no batches
        let not_started = self.services.count_regions_without_batches().await
            .map_err(|e| e.into())? as i32;
        
        // Get all batches by status
        let collecting = self.services.get_batches_by_status(BatchStatus::Collecting).await?.len();
        let ready = self.services.get_batches_by_status(BatchStatus::Ready).await?.len();
        let processing = self.services.get_batches_by_status(BatchStatus::Processing).await?.len();
        let completed = self.services.get_batches_by_status(BatchStatus::Completed).await?.len();
        let failed = self.services.get_batches_by_status(BatchStatus::Failed).await?.len();

        Ok(PipelineStatsResult {
            total_regions,
            not_started,
            fetch_queued: collecting as i32,
            fetching: collecting as i32, // Same as collecting - tasks are in progress
            fetch_failed: failed as i32,
            llm_queued: ready as i32,
            processing: processing as i32,
            done: completed as i32,
            invalidated: 0, // Invalidated is a transient state, not stored separately
        })
    }

    /// Get all configuration entries
    pub async fn get_config(&self) -> Result<Vec<ConfigEntry>, E> {
        self.services.get_all_config().await.map_err(|e| e.into())
    }

    /// Update configuration entries
    pub async fn update_config(&self, entries: Vec<ConfigEntryUpdate>) -> Result<Vec<ConfigEntry>, E> {
        self.services.update_config(entries).await.map_err(|e| e.into())
    }
}
