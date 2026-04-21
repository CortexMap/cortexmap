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
            correlation_id: Some(format!("batch:{}", batch.id)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::{
        NewProcessedFetchTask, OrchConfig, PaperMetadataRecord, ProcessedFetchTask,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use domain::ProcessingBatch;
    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug, thiserror::Error)]
    #[error("mock error: {0}")]
    struct MockErr(String);

    /// Track every mutating call to the fake, so tests can assert the
    /// transitions made by `poll()` / `process()`.
    #[derive(Default)]
    struct Recorder {
        status_updates: Mutex<Vec<(Uuid, BatchStatus, Option<String>)>>,
        completes: Mutex<Vec<Uuid>>,
        cache_dels: Mutex<Vec<String>>,
        cache_del_patterns: Mutex<Vec<String>>,
    }

    struct MockInfra {
        env: HashMap<String, String>,
        config: HashMap<String, String>,
        batches_by_status: Mutex<HashMap<String, Vec<ProcessingBatch>>>,
        completed_count: Mutex<HashMap<Vec<i64>, usize>>,
        s3_keys: Mutex<HashMap<Vec<i64>, Vec<String>>>,
        paper_metadata: Mutex<HashMap<Vec<i64>, Vec<PaperMetadataRecord>>>,
        http_responders: Mutex<HashMap<String, serde_json::Value>>,
        recorder: Recorder,
    }

    impl MockInfra {
        fn new() -> Self {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_string(), "postgres://mock".to_string());
            env.insert(
                "BRAINATLAS_HTTP_ADDR".to_string(),
                "http://brain:8082".to_string(),
            );
            Self {
                env,
                config: HashMap::new(),
                batches_by_status: Mutex::new(HashMap::new()),
                completed_count: Mutex::new(HashMap::new()),
                s3_keys: Mutex::new(HashMap::new()),
                paper_metadata: Mutex::new(HashMap::new()),
                http_responders: Mutex::new(HashMap::new()),
                recorder: Recorder::default(),
            }
        }
        fn with_batch(self, batch: ProcessingBatch) -> Self {
            self.batches_by_status
                .lock()
                .unwrap()
                .entry(batch.status.as_str().to_string())
                .or_default()
                .push(batch);
            self
        }
        fn with_completed_count(self, task_ids: Vec<i64>, count: usize) -> Self {
            let mut key = task_ids;
            key.sort();
            self.completed_count.lock().unwrap().insert(key, count);
            self
        }
    }

    fn mk_batch(
        id: Uuid,
        status: BatchStatus,
        fetch_task_ids: Vec<i64>,
    ) -> ProcessingBatch {
        ProcessingBatch {
            id,
            region_id: Uuid::new_v4(),
            status,
            fetch_task_ids,
            expected_task_count: 0,
            content_hash: None,
            created_at: Utc::now(),
            ready_at: None,
            processing_started_at: None,
            completed_at: None,
            summary_id: None,
            error_message: None,
        }
    }

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
            Ok(self.config.get(&key.to_string()).cloned())
        }

        async fn get_processed_task(
            &self,
            _: &str,
            _: i64,
        ) -> Result<Option<ProcessedFetchTask>, Self::Error> {
            unimplemented!()
        }
        async fn insert_processed_task(
            &self,
            _: &str,
            _: NewProcessedFetchTask,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn update_brainatlas_status(
            &self,
            _: &str,
            _: i64,
            _: &str,
            _: Option<String>,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn get_all_config(
            &self,
            _: &str,
        ) -> Result<Vec<OrchConfig>, Self::Error> {
            unimplemented!()
        }
        async fn update_config(
            &self,
            _: &str,
            _: ConfigKey,
            _: &str,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl HttpClient for MockInfra {
        type Error = MockErr;

        async fn get<T: DeserializeOwned + Send>(&self, url: &str) -> Result<T, Self::Error> {
            let v = self
                .http_responders
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .ok_or_else(|| MockErr(format!("no responder for GET {}", url)))?;
            serde_json::from_value(v).map_err(|e| MockErr(format!("decode: {}", e)))
        }

        async fn post<Req: Serialize + Send + Sync, Res: DeserializeOwned + Send + Sync>(
            &self,
            url: &str,
            _body: &Req,
        ) -> Result<Res, Self::Error> {
            let v = self
                .http_responders
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .ok_or_else(|| MockErr(format!("no responder for POST {}", url)))?;
            serde_json::from_value(v).map_err(|e| MockErr(format!("decode: {}", e)))
        }

        async fn check_health(&self, _: &str, _: &str) -> Result<(), Self::Error> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl BatchManagement for MockInfra {
        type Error = MockErr;

        async fn get_batches_by_status(
            &self,
            _: &str,
            status: BatchStatus,
        ) -> Result<Vec<ProcessingBatch>, Self::Error> {
            Ok(self
                .batches_by_status
                .lock()
                .unwrap()
                .get(status.as_str())
                .cloned()
                .unwrap_or_default())
        }

        async fn count_completed_tasks(
            &self,
            _: &str,
            task_ids: &[i64],
        ) -> Result<usize, Self::Error> {
            let mut key = task_ids.to_vec();
            key.sort();
            Ok(*self
                .completed_count
                .lock()
                .unwrap()
                .get(&key)
                .unwrap_or(&0))
        }

        async fn update_batch_status(
            &self,
            _: &str,
            batch_id: Uuid,
            status: BatchStatus,
            err: Option<String>,
        ) -> Result<(), Self::Error> {
            self.recorder
                .status_updates
                .lock()
                .unwrap()
                .push((batch_id, status, err));
            let mut all = self.batches_by_status.lock().unwrap();
            let mut moved: Option<ProcessingBatch> = None;
            for (_, v) in all.iter_mut() {
                if let Some(pos) = v.iter().position(|b: &ProcessingBatch| b.id == batch_id) {
                    let mut b = v.remove(pos);
                    b.status = status;
                    moved = Some(b);
                    break;
                }
            }
            if let Some(b) = moved {
                all.entry(status.as_str().to_string()).or_default().push(b);
            }
            Ok(())
        }

        async fn complete_batch(
            &self,
            _: &str,
            batch_id: Uuid,
        ) -> Result<(), Self::Error> {
            self.recorder.completes.lock().unwrap().push(batch_id);
            Ok(())
        }

        async fn get_task_s3_keys(
            &self,
            _: &str,
            ids: &[i64],
        ) -> Result<Vec<String>, Self::Error> {
            let mut key = ids.to_vec();
            key.sort();
            Ok(self
                .s3_keys
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .unwrap_or_default())
        }

        async fn get_task_paper_metadata(
            &self,
            _: &str,
            ids: &[i64],
        ) -> Result<Vec<PaperMetadataRecord>, Self::Error> {
            let mut key = ids.to_vec();
            key.sort();
            Ok(self
                .paper_metadata
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .unwrap_or_default())
        }

        async fn get_queries(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Vec<domain::RegionQuery>, Self::Error> {
            unimplemented!()
        }
        async fn insert_queries(
            &self,
            _: &str,
            _: Uuid,
            _: Vec<String>,
        ) -> Result<Vec<Uuid>, Self::Error> {
            unimplemented!()
        }
        async fn delete_queries(&self, _: &str, _: Uuid) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn delete_all_queries(&self, _: &str) -> Result<i64, Self::Error> {
            unimplemented!()
        }
        async fn create_batch(
            &self,
            _: &str,
            _: Uuid,
            _: i32,
        ) -> Result<Uuid, Self::Error> {
            unimplemented!()
        }
        async fn add_tasks_to_batch(
            &self,
            _: &str,
            _: Uuid,
            _: Vec<i64>,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn update_batch_expected_count(
            &self,
            _: &str,
            _: Uuid,
            _: i32,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn get_batch_by_id(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<domain::ProcessingBatch>, Self::Error> {
            unimplemented!()
        }
        async fn get_completed_task_ids(
            &self,
            _: &str,
            _: &[i64],
        ) -> Result<Vec<i64>, Self::Error> {
            unimplemented!()
        }
        async fn get_active_batch(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<ProcessingBatch>, Self::Error> {
            unimplemented!()
        }
        async fn get_recent_batch(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<ProcessingBatch>, Self::Error> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl CacheClient for MockInfra {
        type Error = MockErr;

        async fn cache_get(&self, _key: &str) -> Result<Option<String>, Self::Error> {
            Ok(None)
        }
        async fn cache_set(
            &self,
            _key: &str,
            _val: &str,
            _ttl: u64,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn cache_del(&self, key: &str) -> Result<(), Self::Error> {
            self.recorder
                .cache_dels
                .lock()
                .unwrap()
                .push(key.to_string());
            Ok(())
        }
        async fn cache_del_pattern(&self, pattern: &str) -> Result<u64, Self::Error> {
            self.recorder
                .cache_del_patterns
                .lock()
                .unwrap()
                .push(pattern.to_string());
            Ok(0)
        }
        async fn cache_stats(&self) -> Result<domain::RedisStats, Self::Error> {
            Ok(domain::RedisStats {
                connected: true,
                error: None,
                total_keys: 0,
                keys_by_prefix: vec![],
                used_memory_bytes: 0,
                used_memory_human: "0B".to_string(),
                uptime_secs: 0,
                total_connections_received: 0,
                keyspace_hits: 0,
                keyspace_misses: 0,
                hit_rate: 0.0,
                server_version: "fake".to_string(),
            })
        }
    }

    // ========== TESTS ==========

    // TEST 1: URL normalization helper (parametrized)
    #[test]
    fn normalize_url_handles_all_shapes() {
        type CW = CompletionWatcher<MockInfra>;
        assert_eq!(CW::normalize_url("http://x:8080"), "http://x:8080");
        assert_eq!(CW::normalize_url("https://x"), "https://x");
        assert_eq!(CW::normalize_url("0.0.0.0:8080"), "http://localhost:8080");
        assert_eq!(CW::normalize_url("host:9"), "http://host:9");
        // passthrough branch short-circuits before 0.0.0.0 rewrite
        assert_eq!(
            CW::normalize_url("http://0.0.0.0:8080"),
            "http://0.0.0.0:8080"
        );
    }

    // TEST 2: collecting -> ready when all tasks complete; caches invalidated.
    #[tokio::test]
    async fn poll_promotes_collecting_to_ready_when_all_tasks_complete() {
        let batch_id = Uuid::new_v4();
        let batch = mk_batch(batch_id, BatchStatus::Collecting, vec![1, 2, 3]);
        let infra = Arc::new(
            MockInfra::new()
                .with_batch(batch)
                .with_completed_count(vec![1, 2, 3], 3),
        );
        let cw = CompletionWatcher::new(infra.clone());

        let res = cw.poll().await.expect("poll ok");
        // The batch is now ready; poll returns one PendingTask per fetch_task_id.
        assert_eq!(res.total_found, 2); // already_processed(1) + ready(1)
        assert_eq!(res.already_processed, 1);
        assert_eq!(res.tasks.len(), 3);

        // Assert the status was updated to Ready.
        let updates = infra.recorder.status_updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, batch_id);
        assert_eq!(updates[0].1, BatchStatus::Ready);

        // Expected cache invalidations (single-key + pattern).
        let dels = infra.recorder.cache_dels.lock().unwrap().clone();
        assert!(
            dels.iter()
                .any(|k| k == &crate::cache_keys::batch_status(batch_id))
        );
        assert!(
            dels.iter()
                .any(|k| k == &crate::cache_keys::pipeline_stats())
        );
        let pat = infra.recorder.cache_del_patterns.lock().unwrap().clone();
        assert!(pat.contains(&crate::cache_keys::batches_status_pattern()));
    }

    // TEST 3: collecting stays collecting when not all tasks complete (no-op).
    #[tokio::test]
    async fn poll_noop_when_not_all_tasks_complete() {
        let batch_id = Uuid::new_v4();
        let batch = mk_batch(batch_id, BatchStatus::Collecting, vec![1, 2, 3]);
        let infra = Arc::new(
            MockInfra::new()
                .with_batch(batch)
                .with_completed_count(vec![1, 2, 3], 2), // only 2 of 3
        );
        let cw = CompletionWatcher::new(infra.clone());

        let res = cw.poll().await.expect("poll ok");
        assert_eq!(res.already_processed, 1);
        assert_eq!(res.total_found, 1);
        assert!(res.tasks.is_empty());

        assert!(infra.recorder.status_updates.lock().unwrap().is_empty());
        assert!(infra.recorder.cache_dels.lock().unwrap().is_empty());
        assert!(infra.recorder.cache_del_patterns.lock().unwrap().is_empty());
    }

    // TEST 4: empty-task-id batch is NOT promoted (guard at
    // check_all_tasks_complete).
    #[tokio::test]
    async fn poll_does_not_promote_empty_task_id_batch() {
        let batch_id = Uuid::new_v4();
        let batch = mk_batch(batch_id, BatchStatus::Collecting, vec![]);
        let infra = Arc::new(MockInfra::new().with_batch(batch));
        let cw = CompletionWatcher::new(infra.clone());

        let _ = cw.poll().await.expect("poll ok");
        assert!(infra.recorder.status_updates.lock().unwrap().is_empty());
    }

    // TEST 5: duplicate task_ids dedupe against the distinct count.
    #[tokio::test]
    async fn poll_dedupes_duplicate_task_ids_against_distinct_count() {
        let batch_id = Uuid::new_v4();
        let batch = mk_batch(batch_id, BatchStatus::Collecting, vec![1, 1, 2]);
        let infra = Arc::new(
            MockInfra::new()
                .with_batch(batch)
                .with_completed_count(vec![1, 1, 2], 2),
        );
        let cw = CompletionWatcher::new(infra.clone());
        let _ = cw.poll().await.expect("poll ok");

        let updates = infra.recorder.status_updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1, BatchStatus::Ready);
    }

    // TEST 6: ready batches surface as PendingTasks without further transitions.
    #[tokio::test]
    async fn poll_surfaces_ready_batches_as_pending_tasks() {
        let batch_id = Uuid::new_v4();
        let batch = mk_batch(batch_id, BatchStatus::Ready, vec![10, 20]);
        let infra = Arc::new(MockInfra::new().with_batch(batch));
        let cw = CompletionWatcher::new(infra.clone());

        let res = cw.poll().await.expect("poll ok");
        assert_eq!(res.tasks.len(), 2);
        assert!(infra.recorder.status_updates.lock().unwrap().is_empty());
        for t in &res.tasks {
            assert_eq!(t.pmc_id, format!("batch_{}", batch_id));
            assert!(t.task_id == 10 || t.task_id == 20);
        }
    }
}
