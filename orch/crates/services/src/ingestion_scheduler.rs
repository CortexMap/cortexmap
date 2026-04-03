use crate::{
    BatchManagement, CacheClient, EnvInfra, HttpClient, OrchDatabase, ServiceError,
    GenerateQueriesRequest, GenerateQueriesResponse,
};

/// Typed response from the fetcher-be enqueue endpoint.
/// Using a typed struct (instead of `serde_json::Value`) ensures that missing
/// fields cause a deserialization error rather than silently producing an empty
/// task list, and that the `success` flag is always checked.
#[derive(serde::Deserialize)]
struct EnqueueTaskResponse {
    success: bool,
    task_ids: Vec<i64>,
    #[serde(default)]
    error_message: Option<String>,
}
use crate::cache_keys::{self, invalidate, invalidate_pattern};
use app::{IngestionScheduler, IngestionCycleResult, RegionIngestionResult};
use domain::{BatchStatus, ConfigKey};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

pub struct OrchIngestionScheduler<I> {
    infra: Arc<I>,
}

impl<I> OrchIngestionScheduler<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }

    fn normalize_url(addr: &str) -> String {
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.to_string()
        } else {
            let host_port = addr.replace("0.0.0.0", "localhost");
            format!("http://{}", host_port)
        }
    }
}

#[async_trait::async_trait]
impl<E, I> IngestionScheduler for OrchIngestionScheduler<I>
where
    E: Error + Send + Sync + 'static,
    I: OrchDatabase<Error = E>
        + EnvInfra<Error = E>
        + HttpClient<Error = E>
        + BatchManagement<Error = E>
        + crate::RegionMappingQueries<Error = E>
        + CacheClient<Error = E>
        + Send
        + Sync,
{
    type Error = ServiceError<E>;

    async fn run_ingestion_cycle(&self) -> Result<IngestionCycleResult, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        // Check if ingestion is enabled
        let enabled = self
            .infra
            .get_config(&database_url, ConfigKey::IngestionEnabled)
            .await
            .map_err(ServiceError::InfraError)?
            .map(|v| v == "true")
            .unwrap_or(true);

        if !enabled {
            tracing::info!("Periodic ingestion is disabled, skipping cycle");
            return Ok(IngestionCycleResult {
                regions_processed: 0,
                regions_skipped: 0,
                regions_failed: 0,
                details: vec![],
            });
        }

        // Get batch size config (0 = all regions)
        let batch_size = self
            .infra
            .get_config(&database_url, ConfigKey::IngestionBatchSize)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);

        // Load all regions
        let regions = self
            .infra
            .get_all_regions(&database_url)
            .await
            .map_err(ServiceError::InfraError)?;

        let regions_to_process = if batch_size > 0 {
            &regions[..batch_size.min(regions.len())]
        } else {
            &regions[..]
        };

        tracing::info!(
            total_regions = regions.len(),
            processing = regions_to_process.len(),
            batch_size,
            "Starting ingestion cycle"
        );

        let mut processed = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;
        let mut details = Vec::new();

        for region in regions_to_process {
            match self.ingest_region(region.id).await {
                Ok(result) => {
                    if result.skipped {
                        skipped += 1;
                    } else {
                        processed += 1;
                    }
                    details.push(result);
                }
                Err(e) => {
                    tracing::warn!(
                        region_id = %region.id,
                        region_name = %region.name,
                        error = %e,
                        "Failed to ingest region, continuing with next"
                    );
                    failed += 1;
                    details.push(RegionIngestionResult {
                        region_id: region.id,
                        batch_id: Uuid::nil(),
                        query_count: 0,
                        task_count: 0,
                        skipped: false,
                        skip_reason: Some(format!("Error: {}", e)),
                    });
                }
            }
        }

        tracing::info!(
            processed,
            skipped,
            failed,
            "Ingestion cycle complete"
        );

        Ok(IngestionCycleResult {
            regions_processed: processed,
            regions_skipped: skipped,
            regions_failed: failed,
            details,
        })
    }

    async fn ingest_region(&self, region_id: Uuid) -> Result<RegionIngestionResult, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        // Check if region already has an active batch
        let active_batch = self
            .infra
            .get_active_batch(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)?;

        if active_batch.is_some() {
            return Ok(RegionIngestionResult {
                region_id,
                batch_id: active_batch.unwrap().id,
                query_count: 0,
                task_count: 0,
                skipped: true,
                skip_reason: Some("Active batch already in progress".to_string()),
            });
        }

        // Get region name
        let region_mapping = self
            .infra
            .get_region_mapping(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)?
            .ok_or(ServiceError::NotFound)?;

        let region_name = region_mapping.name;

        // Get query count from config
        let query_count = self
            .infra
            .get_config(&database_url, ConfigKey::QueryGenerationLimit)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(3);

        // Generate queries via brainatlas-be LLM
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

        let url = format!(
            "{}/brainatlas-be/api/generate-queries",
            brainatlas_url.trim_end_matches('/')
        );

        let request = GenerateQueriesRequest {
            region_name: region_name.clone(),
            count: query_count,
        };

        tracing::info!(
            %region_id,
            region_name = %region_name,
            query_count,
            "Generating ingestion queries"
        );

        let response: GenerateQueriesResponse = self
            .infra
            .post(&url, &request)
            .await
            .map_err(ServiceError::InfraError)?;

        let queries = response.queries;
        if queries.is_empty() {
            tracing::warn!(%region_id, "No queries generated, skipping region");
            return Ok(RegionIngestionResult {
                region_id,
                batch_id: Uuid::nil(),
                query_count: 0,
                task_count: 0,
                skipped: true,
                skip_reason: Some("No queries generated by LLM".to_string()),
            });
        }

        // Create batch with a placeholder expected count of 0.
        // The real count (number of papers, not queries) is set unconditionally
        // after the enqueue loop completes so users never see a stale value.
        let batch_id = self
            .infra
            .create_batch(&database_url, region_id, 0)
            .await
            .map_err(ServiceError::InfraError)?;

        // Store queries
        self.infra
            .insert_queries(&database_url, region_id, queries.clone())
            .await
            .map_err(ServiceError::InfraError)?;

        // Enqueue fetch tasks via fetcher-be
        let fetcher_url = match self.infra.get_env_var("FETCHER_HTTP_ADDR") {
            Ok(addr) => Self::normalize_url(&addr),
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

        let mut task_ids = Vec::new();
        for query in &queries {
            let enqueue_req = serde_json::json!({
                "query": query,
                "page_size": 20,
                "max_retry_attempts": 3
            });

            match self
                .infra
                .post::<serde_json::Value, EnqueueTaskResponse>(&enqueue_url, &enqueue_req)
                .await
            {
                Ok(resp) if resp.success => {
                    task_ids.extend(resp.task_ids);
                }
                Ok(resp) => {
                    // Fetcher accepted the request but reported a logical failure
                    // (e.g. NCBI rate-limit, no results). Log and continue so the
                    // remaining queries still run.
                    tracing::warn!(
                        %region_id,
                        query,
                        error = resp.error_message.as_deref().unwrap_or("unknown"),
                        "Fetcher enqueue returned success=false, skipping query"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        %region_id,
                        query,
                        error = %e,
                        "Failed to enqueue fetch task, continuing"
                    );
                }
            }
        }

        let task_count = task_ids.len();

        if !task_ids.is_empty() {
            self.infra
                .add_tasks_to_batch(&database_url, batch_id, task_ids)
                .await
                .map_err(ServiceError::InfraError)?;

            // Always update the expected count to the real number of fetch tasks
            // (papers), not the number of queries used as the initial placeholder.
            self.infra
                .update_batch_expected_count(&database_url, batch_id, task_count as i32)
                .await
                .map_err(ServiceError::InfraError)?;
        } else {
            self.infra
                .update_batch_status(
                    &database_url,
                    batch_id,
                    BatchStatus::Failed,
                    Some("No papers found during ingestion".to_string()),
                )
                .await
                .map_err(ServiceError::InfraError)?;
        }

        // Invalidate caches
        invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;
        invalidate_pattern(self.infra.as_ref(), &cache_keys::batches_status_pattern()).await;

        tracing::info!(
            %region_id,
            %batch_id,
            query_count = queries.len(),
            task_count,
            "Region ingestion enqueued"
        );

        Ok(RegionIngestionResult {
            region_id,
            batch_id,
            query_count: queries.len(),
            task_count,
            skipped: false,
            skip_reason: None,
        })
    }
}
