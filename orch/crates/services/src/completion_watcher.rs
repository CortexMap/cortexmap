use crate::{
    BatchManagement, EnvInfra, HttpClient, OrchDatabase, ProcessRegionRequest,
    ProcessRegionResponse, ServiceError, UuidWrapper,
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
    I: OrchDatabase + EnvInfra + HttpClient + BatchManagement,
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

        // Get ready batches (ignore the tasks parameter since we're batch-based now)
        let ready_batches = self
            .infra
            .get_batches_by_status(&database_url, BatchStatus::Ready)
            .await
            .map_err(ServiceError::InfraError)?;

        let mut successful = 0;
        let mut failed = 0;
        let mut task_results = Vec::new();

        // Process one batch at a time (can be parallelized later)
        for batch in ready_batches.into_iter().take(1) {
            // Limit to 1 batch per process call
            match self
                .process_batch(&batch, &brainatlas_url, &database_url)
                .await
            {
                Ok(detail) => {
                    successful += 1;
                    task_results.push(TaskResult {
                        task_id: batch.fetch_task_ids.first().copied().unwrap_or(0),
                        pmc_id: format!("batch_{}", batch.id),
                        region_id: batch.region_id,
                        status: TaskStatus::Success,
                        detail: Some(detail),
                    });
                }
                Err(e) => {
                    failed += 1;
                    task_results.push(TaskResult {
                        task_id: batch.fetch_task_ids.first().copied().unwrap_or(0),
                        pmc_id: format!("batch_{}", batch.id),
                        region_id: batch.region_id,
                        status: TaskStatus::Failed,
                        detail: Some(e.to_string()),
                    });
                }
            }
        }

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
        + Send
        + Sync,
{
    /// Check if all fetch tasks in a batch are complete
    async fn check_all_tasks_complete(
        &self,
        task_ids: &[i64],
        database_url: &str,
    ) -> Result<bool, ServiceError<E>> {
        let completed_count = self
            .infra
            .count_completed_tasks(database_url, task_ids)
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(completed_count == task_ids.len())
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
                message: "Batch was invalidated".to_string() 
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

        // Get S3 keys directly from database instead of calling fetcher API
        let all_s3_keys = self.infra
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
                    Some("No text files found in batch (only PDFs)".to_string()),
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
            chat_model,
            embedding_model,
        };
        
        tracing::debug!(
            batch_id = %batch.id,
            region_id = %region_uuid,
            s3_keys = ?text_s3_keys,
            "Request payload prepared"
        );

        // Retry with exponential backoff (max 3 attempts)
        let retry_strategy = ExponentialBuilder::default()
            .with_max_times(0)
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
                    .complete_batch(
                        database_url,
                        batch.id,
                    )
                    .await
                    .map_err(ServiceError::InfraError)?;

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

                Err(ServiceError::InfraError(e))
            }
        }
    }
}
