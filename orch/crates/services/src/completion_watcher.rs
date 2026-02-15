use crate::{
    BatchManagement, EnvInfra, HttpClient, OrchDatabase, ProcessRegionRequest,
    ProcessRegionResponse, ServiceError, TaskComponentsResponse,
};
use app::CompletionOrchestrator;
use backon::{ExponentialBuilder, Retryable};
use domain::{BatchStatus, ConfigKey, PendingTask, PollResult, ProcessResult, TaskResult, TaskStatus};
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

        let fetcher_url = match self.infra.get_env_var("FETCHER_URL") {
            Ok(url) => url,
            Err(_) => self
                .infra
                .get_config(&database_url, ConfigKey::FetcherBaseUrl)
                .await
                .map_err(ServiceError::InfraError)?
                .ok_or_else(|| ServiceError::ConfigNotFound {
                    key: "fetcher_base_url".to_string(),
                })?,
        };

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
                .check_all_tasks_complete(&batch.fetch_task_ids, &fetcher_url)
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
                batch.fetch_task_ids.iter().map(move |&task_id| PendingTask {
                    task_id,
                    pmc_id: format!("batch_{}", batch.id), // Placeholder
                    region_id: Uuid::nil(), // Will use batch.region_id instead
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

        let fetcher_url = match self.infra.get_env_var("FETCHER_URL") {
            Ok(url) => url,
            Err(_) => self
                .infra
                .get_config(&database_url, ConfigKey::FetcherBaseUrl)
                .await
                .map_err(ServiceError::InfraError)?
                .ok_or_else(|| ServiceError::ConfigNotFound {
                    key: "fetcher_base_url".to_string(),
                })?,
        };

        let brainatlas_url = match self.infra.get_env_var("BRAINATLAS_URL") {
            Ok(url) => url,
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
                .process_batch(&batch, &fetcher_url, &brainatlas_url, &database_url)
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
        fetcher_url: &str,
    ) -> Result<bool, ServiceError<E>> {
        for &task_id in task_ids {
            let url = format!("{}/api/queue/task/{}/components", fetcher_url, task_id);
            
            // Try to get task details
            match self.infra.get::<TaskComponentsResponse>(&url).await {
                Ok(response) => {
                    // Check if task is complete
                    if response.task_status != "completed" {
                        return Ok(false);
                    }
                }
                Err(_) => {
                    // If we can't get task details, assume it's not complete
                    return Ok(false);
                }
            }
        }
        
        Ok(true)
    }

    /// Process an entire batch: collect all S3 keys and call brainatlas
    async fn process_batch(
        &self,
        batch: &domain::ProcessingBatch,
        fetcher_url: &str,
        brainatlas_url: &str,
        database_url: &str,
    ) -> Result<String, ServiceError<E>> {
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

        // Collect all S3 keys from all fetch tasks in the batch
        let mut all_s3_keys = Vec::new();
        
        for &task_id in &batch.fetch_task_ids {
            let components_url = format!("{}/api/queue/task/{}/components", fetcher_url, task_id);
            let components: TaskComponentsResponse = self
                .infra
                .get(&components_url)
                .await
                .map_err(ServiceError::InfraError)?;

            // Extract S3 keys
            let s3_keys: Vec<String> = components
                .components
                .iter()
                .filter_map(|c| c.s3_key.clone())
                .collect();

            all_s3_keys.extend(s3_keys);
        }

        if all_s3_keys.is_empty() {
            self.infra
                .update_batch_status(
                    database_url,
                    batch.id,
                    BatchStatus::Failed,
                    Some("No S3 keys found in batch".to_string()),
                )
                .await
                .map_err(ServiceError::InfraError)?;
            return Err(ServiceError::NoS3Keys);
        }

        tracing::info!(
            batch_id = %batch.id,
            s3_key_count = all_s3_keys.len(),
            "Collected S3 keys from batch"
        );

        // Call brainatlas /process with retry logic
        let process_url = format!("{}/api/process", brainatlas_url);
        
        let region_uuid = batch.region_id;
        
        let request = ProcessRegionRequest {
            region_id: region_uuid.to_string(),
            s3_keys: all_s3_keys.clone(),
        };

        // Retry with exponential backoff (max 3 attempts)
        let retry_strategy = ExponentialBuilder::default()
            .with_max_times(3)
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
                // Mark batch as complete with summary_id and content_hash from response
                self.infra
                    .complete_batch(
                        database_url,
                        batch.id,
                        response.summary_id,
                        response.content_hash,
                    )
                    .await
                    .map_err(ServiceError::InfraError)?;

                tracing::info!(
                    batch_id = %batch.id,
                    summary_id = %response.summary_id,
                    "Batch processing completed successfully"
                );

                Ok(response.detail)
            }
            Err(e) => {
                self.infra
                    .update_batch_status(
                        database_url,
                        batch.id,
                        BatchStatus::Failed,
                        Some(e.to_string()),
                    )
                    .await
                    .map_err(ServiceError::InfraError)?;

                tracing::error!(
                    batch_id = %batch.id,
                    error = %e,
                    "Batch processing failed"
                );

                Err(ServiceError::InfraError(e))
            }
        }
    }
}

