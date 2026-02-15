use crate::{
    EnvInfra, HttpClient, NewProcessedFetchTask, OrchDatabase, ProcessRegionRequest,
    ProcessRegionResponse, ServiceError, TaskComponentsResponse, TaskDetailsResponse,
};
use app::CompletionOrchestrator;
use domain::{ConfigKey, PendingTask, PollResult, ProcessResult, TaskResult, TaskStatus};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

pub struct CompletionWatcher<I> {
    infra: Arc<I>,
}

impl<I> CompletionWatcher<I>
where
    I: OrchDatabase + EnvInfra + HttpClient,
{
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<E, I> CompletionOrchestrator for CompletionWatcher<I>
where
    E: Error + Send + Sync + 'static,
    I: OrchDatabase<Error = E> + EnvInfra<Error = E> + HttpClient<Error = E> + Send + Sync,
{
    type Error = ServiceError<E>;

    async fn poll(&self) -> Result<PollResult, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        // Try env var first, then fall back to DB config
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

        let max_parallel = self
            .infra
            .get_config(&database_url, ConfigKey::MaxParallelProcessCalls)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        // Get completed fetch tasks from fetcher API
        let url = format!(
            "{}/api/queue/tasks?status=completed&limit={}",
            fetcher_url, max_parallel
        );
        let tasks: Vec<TaskDetailsResponse> = self
            .infra
            .get(&url)
            .await
            .map_err(ServiceError::InfraError)?;

        let total_found = tasks.len();
        let mut already_processed = 0;
        let mut pending_tasks = Vec::new();

        // Filter out already processed and extract needed data
        for task in tasks {
            if let Some(task_id) = task.task_id {
                // Check if already processed
                if self
                    .infra
                    .get_processed_task(&database_url, task_id)
                    .await
                    .map_err(ServiceError::InfraError)?
                    .is_some()
                {
                    already_processed += 1;
                } else {
                    // FIXME: For now, we don't have region_id in the response
                    // This needs to be added to the fetcher API or we need a mapping
                    // For now, use a placeholder UUID - this needs to be fixed
                    let region_id = Uuid::nil(); // TODO: Get actual region_id
                    pending_tasks.push(PendingTask {
                        task_id,
                        pmc_id: task.pmc_id,
                        region_id,
                    });
                }
            }
        }

        Ok(PollResult {
            tasks: pending_tasks,
            total_found,
            already_processed,
        })
    }

    async fn process(&self, tasks: Vec<PendingTask>) -> Result<ProcessResult, Self::Error> {
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

        let mut successful = 0;
        let mut failed = 0;
        let mut task_results = Vec::new();

        for task in tasks {
            match self
                .process_single_task(&task, &fetcher_url, &brainatlas_url, &database_url)
                .await
            {
                Ok(detail) => {
                    successful += 1;
                    task_results.push(TaskResult {
                        task_id: task.task_id,
                        pmc_id: task.pmc_id.clone(),
                        region_id: task.region_id,
                        status: TaskStatus::Success,
                        detail: Some(detail),
                    });
                }
                Err(e) => {
                    failed += 1;
                    task_results.push(TaskResult {
                        task_id: task.task_id,
                        pmc_id: task.pmc_id.clone(),
                        region_id: task.region_id,
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
    I: OrchDatabase<Error = E> + EnvInfra<Error = E> + HttpClient<Error = E> + Send + Sync,
{
    async fn process_single_task(
        &self,
        task: &PendingTask,
        fetcher_url: &str,
        brainatlas_url: &str,
        database_url: &str,
    ) -> Result<String, ServiceError<E>> {
        // Double-check if already processed (race condition guard)
        if self
            .infra
            .get_processed_task(database_url, task.task_id)
            .await
            .map_err(ServiceError::InfraError)?
            .is_some()
        {
            return Err(ServiceError::AlreadyProcessed);
        }

        // Insert with status='pending'
        let new_task = NewProcessedFetchTask {
            fetch_task_id: task.task_id,
            region_id: task.region_id,
            brainatlas_status: "pending".to_string(),
        };
        self.infra
            .insert_processed_task(database_url, new_task)
            .await
            .map_err(ServiceError::InfraError)?;

        // Get components from fetcher
        let components_url = format!("{}/api/queue/task/{}/components", fetcher_url, task.task_id);
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

        if s3_keys.is_empty() {
            self.infra
                .update_brainatlas_status(
                    database_url,
                    task.task_id,
                    "failed",
                    Some("No S3 keys found".to_string()),
                )
                .await
                .map_err(ServiceError::InfraError)?;
            return Err(ServiceError::NoS3Keys);
        }

        // Update status to in_progress
        self.infra
            .update_brainatlas_status(database_url, task.task_id, "in_progress", None)
            .await
            .map_err(ServiceError::InfraError)?;

        // Call brainatlas /process
        let process_url = format!("{}/api/process", brainatlas_url);
        let request = ProcessRegionRequest {
            region_id: task.region_id.to_string(),
            s3_keys,
        };

        match self
            .infra
            .post::<ProcessRegionRequest, ProcessRegionResponse>(&process_url, &request)
            .await
        {
            Ok(response) => {
                self.infra
                    .update_brainatlas_status(database_url, task.task_id, "completed", None)
                    .await
                    .map_err(ServiceError::InfraError)?;
                Ok(response.detail)
            }
            Err(e) => {
                self.infra
                    .update_brainatlas_status(
                        database_url,
                        task.task_id,
                        "failed",
                        Some(e.to_string()),
                    )
                    .await
                    .map_err(ServiceError::InfraError)?;
                Err(ServiceError::InfraError(e))
            }
        }
    }
}

