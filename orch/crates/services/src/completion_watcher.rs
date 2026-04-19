use crate::cache_keys::{self, invalidate, invalidate_pattern};
use crate::{
    BatchManagement, CacheClient, EnvInfra, HttpClient, OrchDatabase, PaperMetadataEntry,
    ProcessRegionRequest, ProcessRegionResponse, ServiceError, UuidWrapper,
};
use app::CompletionOrchestrator;
use backon::{ExponentialBuilder, Retryable};
use domain::{
    BatchStatus, ConfigKey, PendingTask, PollResult, ProcessResult, TaskResult, TaskStatus,
};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

pub struct CompletionWatcher<I> {
    infra: Arc<I>,
}

impl<I> CompletionWatcher<I>
where
    I: OrchDatabase + EnvInfra + HttpClient + BatchManagement + CacheClient,
{
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }

    /// Normalize HTTP address to full URL
    /// Converts "0.0.0.0:8080" to "http://localhost:8080"
    /// Passes through URLs that already have protocol
    fn normalize_url(addr: &str) -> String {
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.to_string()
        } else {
            // Replace 0.0.0.0 with localhost and add http://
            let host_port = addr.replace("0.0.0.0", "localhost");
            format!("http://{}", host_port)
        }
    }
}

#[async_trait::async_trait]
impl<E, I> CompletionOrchestrator for CompletionWatcher<I>
where
    E: Error + Send + Sync + 'static,
    I: OrchDatabase<Error = E>
        + EnvInfra<Error = E>
        + HttpClient<Error = E>
        + BatchManagement<Error = E>
        + CacheClient<Error = E>
        + Send
        + Sync,
{
    type Error = ServiceError<E>;

    async fn poll(&self) -> Result<PollResult, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        // Get batches in 'collecting' status
        let collecting_batches = self
            .infra
            .get_batches_by_status(&database_url, BatchStatus::Collecting)
            .await
            .map_err(ServiceError::InfraError)?;

        let already_processed = collecting_batches.len();

        // For each batch, check if all its fetch tasks are complete
        for batch in collecting_batches {
            let all_complete = self
                .check_all_tasks_complete(&batch.fetch_task_ids, &database_url)
                .await?;

            if all_complete {
                // Mark batch as ready
                self.infra
                    .update_batch_status(&database_url, batch.id, BatchStatus::Ready, None)
                    .await
                    .map_err(ServiceError::InfraError)?;

                // Invalidate caches affected by batch status change
                invalidate(self.infra.as_ref(), &cache_keys::batch_status(batch.id)).await;
                invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;
                invalidate_pattern(self.infra.as_ref(), &cache_keys::batches_status_pattern())
                    .await;

                tracing::info!(
                    batch_id = %batch.id,
                    region_id = %batch.region_id,
                    task_count = batch.fetch_task_ids.len(),
                    "Batch ready for processing"
                );
            }
        }

        // Get batches that are ready to process
        let ready_batches = self
            .infra
            .get_batches_by_status(&database_url, BatchStatus::Ready)
            .await
            .map_err(ServiceError::InfraError)?;

        // Convert ready batches to PendingTask format for backward compatibility
        let pending_tasks: Vec<PendingTask> = ready_batches
            .iter()
            .flat_map(|batch| {
                // For each batch, create one PendingTask per fetch task
                // In practice, we'll process the whole batch together
                batch
                    .fetch_task_ids
                    .iter()
                    .map(move |&task_id| PendingTask {
                        task_id,
                        pmc_id: format!("batch_{}", batch.id), // Placeholder
                        region_id: Uuid::nil(),                // Will use batch.region_id instead
                    })
            })
            .collect();

        Ok(PollResult {
            tasks: pending_tasks,
            total_found: ready_batches.len() + already_processed,
            already_processed,
        })
    }

    async fn process(&self, _tasks: Vec<PendingTask>) -> Result<ProcessResult, Self::Error> {
        use futures::stream::{self, StreamExt};

        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let brainatlas_url = match self.infra.get_env_var("BRAINATLAS_HTTP_ADDR") {
            Ok(addr) => Self::normalize_url(&addr),
            Err(_) => self
                .infra
                .get_config(&database_url, ConfigKey::BrainatlasBaseUrl)
                .await
                .map_err(ServiceError::InfraError)?
                .ok_or_else(|| ServiceError::ConfigNotFound {
                    key: "brainatlas_base_url".to_string(),
                })?,
        };

        // Read max parallel LLM calls from config (default 10).
        // This bounds concurrent brainatlas /process calls so we don't overwhelm
        // the LLM provider while still getting significant throughput gains over
        // sequential processing.
        let concurrency: usize = self
            .infra
            .get_config(&database_url, ConfigKey::MaxParallelProcessCalls)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        // Get ready batches (ignore the tasks parameter since we're batch-based now)
        let ready_batches = self
            .infra
            .get_batches_by_status(&database_url, BatchStatus::Ready)
            .await
            .map_err(ServiceError::InfraError)?;

        let total_ready = ready_batches.len();
        tracing::info!(
            ready_batches = total_ready,
            concurrency = concurrency,
            "Processing ready batches in parallel"
        );

        // Process batches in parallel with bounded concurrency using buffer_unordered.
        // Each batch's process_batch is independent — it writes to its own batch row
        // and region_summary row — so parallelism is safe.
        let brainatlas_url_ref = &brainatlas_url;
        let database_url_ref = &database_url;

        let results: Vec<(uuid::Uuid, i64, uuid::Uuid, Result<String, ServiceError<E>>)> =
            stream::iter(ready_batches)
                .map(|batch| async move {
                    let batch_id = batch.id;
                    let region_id = batch.region_id;
                    let first_task_id = batch.fetch_task_ids.first().copied().unwrap_or(0);
                    let result = self
                        .process_batch(&batch, brainatlas_url_ref, database_url_ref)
                        .await;
                    (batch_id, first_task_id, region_id, result)
                })
                .buffer_unordered(concurrency)
                .collect()
                .await;

        let mut successful = 0;
        let mut failed = 0;
        let mut task_results = Vec::with_capacity(results.len());

        for (batch_id, task_id, region_id, result) in results {
            match result {
                Ok(detail) => {
                    successful += 1;
                    task_results.push(TaskResult {
                        task_id,
                        pmc_id: format!("batch_{}", batch_id),
                        region_id,
                        status: TaskStatus::Success,
                        detail: Some(detail),
                    });
                }
                Err(e) => {
                    failed += 1;
                    task_results.push(TaskResult {
                        task_id,
                        pmc_id: format!("batch_{}", batch_id),
                        region_id,
                        status: TaskStatus::Failed,
                        detail: Some(e.to_string()),
                    });
                }
            }
        }

        tracing::info!(
            successful = successful,
            failed = failed,
            total = total_ready,
            "Parallel batch processing complete"
        );

        Ok(ProcessResult {
            successful,
            failed,
            task_results,
        })
    }

    async fn get_config(&self, key: ConfigKey) -> Result<Option<String>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_config(&database_url, key)
            .await
            .map_err(ServiceError::InfraError)
    }
}

impl<E, I> CompletionWatcher<I>
where
    E: Error + Send + Sync + 'static,
    I: OrchDatabase<Error = E>
        + EnvInfra<Error = E>
        + HttpClient<Error = E>
        + BatchManagement<Error = E>
        + CacheClient<Error = E>
        + Send
        + Sync,
{
    /// Check if all fetch tasks in a batch are complete.
    ///
    /// Returns `false` when `task_ids` is empty — an empty batch must never be
    /// promoted to `ready`, it should be caught upstream and marked `failed`.
    async fn check_all_tasks_complete(
        &self,
        task_ids: &[i64],
        database_url: &str,
    ) -> Result<bool, ServiceError<E>> {
        // Guard: a batch with no tasks is not "complete" — it is broken.
        // Without this guard, 0 == 0 would be vacuously true and the watcher
        // would immediately promote the batch to `ready` with nothing to process.
        if task_ids.is_empty() {
            return Ok(false);
        }

        // Deduplicate task_ids before comparing. The `fetch_task_ids` array can
        // contain duplicates (e.g. when ON CONFLICT(pmc_id) returns the same
        // existing task ID for multiple queries that found the same paper).
        // count_completed_tasks does `id IN (...)` which is set-semantic, so we
        // must compare against the *distinct* count, not the array length.
        let unique_ids: std::collections::HashSet<i64> = task_ids.iter().copied().collect();

        let completed_count = self
            .infra
            .count_completed_tasks(database_url, task_ids)
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(completed_count == unique_ids.len())
    }

    /// Process an entire batch: collect all S3 keys and call brainatlas
    async fn process_batch(
        &self,
        batch: &domain::ProcessingBatch,
        brainatlas_url: &str,
        database_url: &str,
    ) -> Result<String, ServiceError<E>> {
        // Check if batch was invalidated (skip processing if so)
        if batch.status == BatchStatus::Invalidated {
            tracing::info!(
                batch_id = %batch.id,
                "Skipping invalidated batch"
            );
            return Err(ServiceError::External {
                message: "Batch was invalidated".to_string(),
            });
        }

        tracing::info!(
            batch_id = %batch.id,
            region_id = %batch.region_id,
            task_count = batch.fetch_task_ids.len(),
            "Processing batch"
        );

        // Mark batch as processing
        self.infra
            .update_batch_status(database_url, batch.id, BatchStatus::Processing, None)
            .await
            .map_err(ServiceError::InfraError)?;

        // Invalidate caches affected by batch status change
        invalidate(self.infra.as_ref(), &cache_keys::batch_status(batch.id)).await;
        invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;
        invalidate_pattern(self.infra.as_ref(), &cache_keys::batches_status_pattern()).await;

        // Detect zombie batches: no fetch tasks were ever assigned.
        // This is distinct from "tasks exist but produced only PDFs".
        if batch.fetch_task_ids.is_empty() {
            tracing::error!(
                batch_id = %batch.id,
                region_id = %batch.region_id,
                "Zombie batch: fetch_task_ids is empty — no tasks were ever assigned. Marking failed."
            );
            self.infra
                .update_batch_status(
                    database_url,
                    batch.id,
                    BatchStatus::Failed,
                    Some("No fetch tasks were ever assigned to this batch".to_string()),
                )
                .await
                .map_err(ServiceError::InfraError)?;
            return Err(ServiceError::NoS3Keys);
        }

        // Get S3 keys directly from database instead of calling fetcher API
        let all_s3_keys = self
            .infra
            .get_task_s3_keys(database_url, &batch.fetch_task_ids)
            .await
            .map_err(ServiceError::InfraError)?;

        // Filter out PDF files (binary content that brainatlas can't process as UTF-8)
        let text_s3_keys: Vec<String> = all_s3_keys
            .iter()
            .filter(|key| {
                let lower_key = key.to_lowercase();
                !lower_key.ends_with(".pdf")
            })
            .cloned()
            .collect();

        if text_s3_keys.is_empty() {
            self.infra
                .update_batch_status(
                    database_url,
                    batch.id,
                    BatchStatus::Failed,
                    Some(format!(
                        "No text files found in batch ({} S3 keys were all PDFs or unavailable)",
                        all_s3_keys.len()
                    )),
                )
                .await
                .map_err(ServiceError::InfraError)?;
            return Err(ServiceError::NoS3Keys);
        }

        tracing::info!(
            batch_id = %batch.id,
            total_keys = all_s3_keys.len(),
            text_keys = text_s3_keys.len(),
            "Filtered S3 keys (excluding PDFs)"
        );

        // Get paper metadata for source attribution
        let paper_metadata_records = self
            .infra
            .get_task_paper_metadata(database_url, &batch.fetch_task_ids)
            .await
            .map_err(ServiceError::InfraError)?;

        // Build paper_metadata entries, filtering to only text S3 keys
        let paper_metadata: Vec<PaperMetadataEntry> = paper_metadata_records
            .into_iter()
            .filter(|r| text_s3_keys.contains(&r.s3_key))
            .map(|r| PaperMetadataEntry {
                s3_key: r.s3_key,
                pmc_id: r.pmc_id,
                uid: r.uid,
                query: r.query,
            })
            .collect();

        tracing::info!(
            batch_id = %batch.id,
            paper_metadata_count = paper_metadata.len(),
            "Collected paper metadata for source attribution"
        );

        // Call brainatlas /process with retry logic
        let process_url = format!("{}/brainatlas-be/api/process", brainatlas_url);

        tracing::info!(
            batch_id = %batch.id,
            url = %process_url,
            "Calling brainatlas process endpoint"
        );

        let region_uuid = batch.region_id;

        // Read model configuration from orch config (with env var fallback)
        let chat_model = match self.infra.get_env_var("CHAT_MODEL") {
            Ok(model) => Some(model),
            Err(_) => self
                .infra
                .get_config(database_url, ConfigKey::ChatModel)
                .await
                .map_err(ServiceError::InfraError)?,
        };

        let embedding_model = match self.infra.get_env_var("EMBEDDING_MODEL") {
            Ok(model) => Some(model),
            Err(_) => self
                .infra
                .get_config(database_url, ConfigKey::EmbeddingModel)
                .await
                .map_err(ServiceError::InfraError)?,
        };

        let request = ProcessRegionRequest {
            region_id: UuidWrapper {
                value: region_uuid.to_string(),
            },
            batch_id: UuidWrapper {
                value: batch.id.to_string(),
            },
            s3_keys: text_s3_keys.clone(),
            paper_metadata,
            chat_model,
            embedding_model,
            skip_summarization: false,
        };

        tracing::debug!(
            batch_id = %batch.id,
            region_id = %region_uuid,
            s3_keys = ?text_s3_keys,
            "Request payload prepared"
        );

        // Retry with exponential backoff (max 3 attempts)
        let retry_strategy = ExponentialBuilder::default()
            .with_max_times(2)
            .with_min_delay(std::time::Duration::from_secs(1))
            .with_max_delay(std::time::Duration::from_secs(10));

        let infra = Arc::clone(&self.infra);
        let url_clone = process_url.clone();
        let req_clone = request.clone();

        let result = (|| async {
            infra
                .post::<ProcessRegionRequest, ProcessRegionResponse>(&url_clone, &req_clone)
                .await
        })
        .retry(retry_strategy)
        .await;

        match result {
            Ok(response) => {
                // Mark batch as complete
                self.infra
                    .complete_batch(database_url, batch.id)
                    .await
                    .map_err(ServiceError::InfraError)?;

                // Invalidate caches affected by batch completion
                let region_id = batch.region_id;
                invalidate(
                    self.infra.as_ref(),
                    &cache_keys::region_summaries(region_id),
                )
                .await;
                invalidate(self.infra.as_ref(), &cache_keys::region_status(region_id)).await;
                invalidate(self.infra.as_ref(), &cache_keys::batch_status(batch.id)).await;
                invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;
                invalidate(self.infra.as_ref(), &cache_keys::all_regions()).await;
                invalidate_pattern(self.infra.as_ref(), &cache_keys::batches_status_pattern())
                    .await;

                tracing::info!(
                    batch_id = %batch.id,
                    detail = %response.detail,
                    "Batch processing completed successfully"
                );

                Ok(response.detail)
            }
            Err(e) => {
                tracing::error!(
                    batch_id = %batch.id,
                    error = %e,
                    "Batch processing failed"
                );

                self.infra
                    .update_batch_status(
                        database_url,
                        batch.id,
                        BatchStatus::Failed,
                        Some(e.to_string()),
                    )
                    .await
                    .map_err(ServiceError::InfraError)?;

                // Invalidate caches affected by batch failure
                invalidate(self.infra.as_ref(), &cache_keys::batch_status(batch.id)).await;
                invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;
                invalidate_pattern(self.infra.as_ref(), &cache_keys::batches_status_pattern())
                    .await;

                Err(ServiceError::InfraError(e))
            }
        }
    }
}
