use crate::{HttpClient, Infra, NewProcessedFetchTask};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

pub struct CompletionWatcher<I> {
    infra: Arc<I>,
    http_client: HttpClient,
}

impl<I: Infra> CompletionWatcher<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self {
            infra,
            http_client: HttpClient::new(),
        }
    }

    /// Run the completion watcher loop
    pub async fn run(self) {
        loop {
            if let Err(e) = self.poll_and_process().await {
                eprintln!("[CompletionWatcher] Error: {}", e);
            }
            
            // Get poll interval from config, default to 30 seconds
            let interval = self.get_poll_interval().await.unwrap_or(30);
            sleep(Duration::from_secs(interval)).await;
        }
    }

    async fn poll_and_process(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Get config
        let database_url = self.infra.get("DATABASE_URL")?;
        let fetcher_url = self.infra.get("FETCHER_URL")
            .or_else(|_| {
                self.infra
                    .get_config(&database_url, "fetcher_base_url")
                    .await?
                    .ok_or_else(|| "fetcher_base_url not configured".to_string())
            })?;
        let brainatlas_url = self.infra.get("BRAINATLAS_URL")
            .or_else(|_| {
                self.infra
                    .get_config(&database_url, "brainatlas_base_url")
                    .await?
                    .ok_or_else(|| "brainatlas_base_url not configured".to_string())
            })?;
        
        let max_parallel = self.infra
            .get_config(&database_url, "max_parallel_process_calls")
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        // Get completed fetch tasks
        let tasks = self.http_client
            .get_completed_tasks(&fetcher_url, max_parallel)
            .await?;

        println!("[CompletionWatcher] Found {} completed tasks", tasks.len());

        // Filter out already processed
        let mut new_tasks = Vec::new();
        for task in tasks {
            // Extract task_id from the task (we need to add this field)
            // For now, we'll skip this - we need task_id from fetcher response
            // TODO: Update FetchTask struct to include task_id
        }

        // Process each task
        for (task_id, pmc_id, region_id) in new_tasks {
            if let Err(e) = self.process_single_task(task_id, pmc_id, region_id, &fetcher_url, &brainatlas_url, &database_url).await {
                eprintln!("[CompletionWatcher] Failed to process task {}: {}", task_id, e);
            }
        }

        Ok(())
    }

    async fn process_single_task(
        &self,
        task_id: i64,
        pmc_id: String,
        region_id: Uuid,
        fetcher_url: &str,
        brainatlas_url: &str,
        database_url: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Check if already processed
        if self.infra.get_processed_task(database_url, task_id).await?.is_some() {
            return Ok(());
        }

        // Insert with status='pending'
        let new_task = NewProcessedFetchTask {
            fetch_task_id: task_id,
            region_id,
            brainatlas_status: "pending".to_string(),
        };
        self.infra.insert_processed_task(database_url, new_task).await?;

        // Get components from fetcher
        let components = self.http_client
            .get_task_components(fetcher_url, task_id)
            .await?;

        // Extract S3 keys
        let s3_keys: Vec<String> = components
            .components
            .iter()
            .filter_map(|c| c.s3_key.clone())
            .collect();

        if s3_keys.is_empty() {
            eprintln!("[CompletionWatcher] No S3 keys found for task {}", task_id);
            self.infra
                .update_brainatlas_status(database_url, task_id, "failed", Some("No S3 keys found".to_string()))
                .await?;
            return Ok(());
        }

        // Update status to in_progress
        self.infra
            .update_brainatlas_status(database_url, task_id, "in_progress", None)
            .await?;

        // Call brainatlas /process
        match self.http_client
            .process_region(brainatlas_url, region_id, s3_keys)
            .await
        {
            Ok(response) => {
                println!("[CompletionWatcher] Successfully processed task {}: {}", task_id, response.detail);
                self.infra
                    .update_brainatlas_status(database_url, task_id, "completed", None)
                    .await?;
            }
            Err(e) => {
                eprintln!("[CompletionWatcher] Brainatlas processing failed for task {}: {}", task_id, e);
                self.infra
                    .update_brainatlas_status(database_url, task_id, "failed", Some(e.to_string()))
                    .await?;
            }
        }

        Ok(())
    }

    async fn get_poll_interval(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let database_url = self.infra.get("DATABASE_URL")?;
        let interval = self.infra
            .get_config(&database_url, "completion_poll_interval_secs")
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        Ok(interval)
    }
}
