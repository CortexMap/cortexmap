use crate::{BatchManagement, CacheClient, EnvInfra, HttpClient, ServiceError, GenerateQueriesRequest, GenerateQueriesResponse};
use crate::cache_keys::{self, cached_or_fetch, invalidate, invalidate_pattern};
use app::RegionManagement;
use domain::{BatchStatus, ChunkSourceResponse, ConfigKey, ProcessingBatch, RegionQuery, RegionSummary, SummarySource};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

pub struct OrchRegionManagement<I> {
    infra: Arc<I>,
}

impl<I> OrchRegionManagement<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<E, I> RegionManagement for OrchRegionManagement<I>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E> + HttpClient<Error = E> + BatchManagement<Error = E> + crate::OrchDatabase<Error = E> + crate::RegionMappingQueries<Error = E> + CacheClient<Error = E> + Send + Sync,
{
    type Error = ServiceError<E>;

    async fn get_summaries(&self, region_id: Uuid) -> Result<Vec<RegionSummary>, Self::Error> {
        let infra = &self.infra;
        cached_or_fetch(
            infra.as_ref(),
            &cache_keys::region_summaries(region_id),
            cache_keys::TTL_MEDIUM,
            || async {
                let database_url = infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;

                // First get the region mapping to find the Int4 region_id
                let region_mapping = infra
                    .get_region_mapping(&database_url, region_id)
                    .await
                    .map_err(ServiceError::InfraError)?
                    .ok_or_else(|| ServiceError::NotFound)?;

                // Query the region_summary table directly
                let summaries = infra
                    .get_region_summaries(&database_url, region_mapping.region_id)
                    .await
                    .map_err(ServiceError::InfraError)?;

                // For each summary, fetch its source chunks
                let mut result = Vec::with_capacity(summaries.len());
                for s in summaries {
                    let sources = infra
                        .get_summary_sources(&database_url, s.id)
                        .await
                        .map_err(ServiceError::InfraError)?;

                    result.push(RegionSummary {
                        summary: s.summary.unwrap_or_default(),
                        created_at: chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                            s.created_at,
                            chrono::Utc,
                        ),
                        batch_id: s.batch_id,
                        sources: sources
                            .into_iter()
                            .map(|src| SummarySource {
                                chunk_id: src.id,
                                pmc_id: src.source_pmc_id,
                                uid: src.source_uid,
                                source_query: src.source_query,
                            })
                            .collect(),
                    });
                }

                Ok(result)
            },
        )
        .await
    }

    async fn get_active_batch(
        &self,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_active_batch(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn get_recent_batch(
        &self,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_recent_batch(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn get_queries(&self, region_id: Uuid) -> Result<Vec<RegionQuery>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_queries(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn update_batch_status(
        &self,
        batch_id: Uuid,
        status: BatchStatus,
        error_message: Option<String>,
    ) -> Result<(), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .update_batch_status(&database_url, batch_id, status, error_message)
            .await
            .map_err(ServiceError::InfraError)?;

        // Invalidate batch-related caches
        invalidate(self.infra.as_ref(), &cache_keys::batch_status(batch_id)).await;
        invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;
        invalidate_pattern(self.infra.as_ref(), &cache_keys::batches_status_pattern()).await;

        Ok(())
    }

    async fn store_queries(
        &self,
        region_id: Uuid,
        queries: Vec<String>,
    ) -> Result<Vec<Uuid>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .insert_queries(&database_url, region_id, queries)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn generate_queries(&self, region_name: &str, count: u32) -> Result<Vec<String>, Self::Error> {
        tracing::info!(region_name, count, "Generating queries using LLM via brainatlas");
        
        // Normalize URL helper
        fn normalize_url(addr: &str) -> String {
            if addr.starts_with("http://") || addr.starts_with("https://") {
                addr.to_string()
            } else {
                let replaced = addr.replace("0.0.0.0", "localhost");
                format!("http://{}", replaced)
            }
        }
        
        // Get brainatlas URL from env or config
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;
        
        let brainatlas_url = match self.infra.get_env_var("BRAINATLAS_HTTP_ADDR") {
            Ok(addr) => normalize_url(&addr),
            Err(_) => {
                self.infra
                    .get_config(&database_url, ConfigKey::BrainatlasBaseUrl)
                    .await
                    .map_err(ServiceError::InfraError)?
                    .ok_or_else(|| ServiceError::ConfigNotFound {
                        key: "brainatlas_base_url".to_string(),
                    })?
            }
        };
        
        let url = format!("{}/brainatlas-be/api/generate-queries", brainatlas_url.trim_end_matches('/'));
        
        let request = GenerateQueriesRequest {
            region_name: region_name.to_string(),
            count,
        };
        
        tracing::info!(url = %url, region_name, count, "Calling brainatlas generate-queries endpoint");
        
        let response: GenerateQueriesResponse = self.infra
            .post(&url, &request)
            .await
            .map_err(ServiceError::InfraError)?;
        
        tracing::info!(
            region_name,
            query_count = response.queries.len(),
            queries = ?response.queries,
            "Successfully generated LLM queries"
        );
        
        Ok(response.queries)
    }
    
    async fn get_batches_by_status(
        &self,
        status: BatchStatus,
    ) -> Result<Vec<ProcessingBatch>, Self::Error> {
        let infra = &self.infra;
        let key = cache_keys::batches_by_status(status.as_str());
        cached_or_fetch(
            infra.as_ref(),
            &key,
            cache_keys::TTL_SHORT,
            || async {
                let database_url = infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;

                infra
                    .get_batches_by_status(&database_url, status)
                    .await
                    .map_err(ServiceError::InfraError)
            },
        )
        .await
    }
    
    async fn get_region_name(&self, region_id: Uuid) -> Result<String, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let region = self
            .infra
            .get_region_mapping(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)?
            .ok_or_else(|| ServiceError::NotFound)?;

        Ok(region.name)
    }
    
    async fn get_total_regions(&self) -> Result<i64, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_total_region_count(&database_url)
            .await
            .map_err(ServiceError::InfraError)
    }
    
    async fn count_regions_without_batches(&self) -> Result<i64, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .count_regions_without_batches(&database_url)
            .await
            .map_err(ServiceError::InfraError)
    }
    
    async fn get_query_generation_limit(&self) -> Result<Option<u32>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let value = self
            .infra
            .get_config(&database_url, ConfigKey::QueryGenerationLimit)
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(value.and_then(|v| v.parse::<u32>().ok()))
    }
    
    async fn get_all_regions(&self) -> Result<Vec<domain::Region>, Self::Error> {
        let infra = &self.infra;
        cached_or_fetch(
            infra.as_ref(),
            &cache_keys::all_regions(),
            cache_keys::TTL_LONG,
            || async {
                let database_url = infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;

                let regions = infra
                    .get_all_regions(&database_url)
                    .await
                    .map_err(ServiceError::InfraError)?;

                Ok(regions
                    .into_iter()
                    .map(|r| domain::Region {
                        id: r.id,
                        region_id: r.region_id,
                        name: r.name,
                        acronym: r.acronym,
                        color: if let (Some(red), Some(green), Some(blue)) = (r.red, r.green, r.blue) {
                            Some(domain::RegionColor { red, green, blue })
                        } else {
                            None
                        },
                        structure_order: r.structure_order,
                        parent_region_id: r.parent_region_id,
                        parent_acronym: r.parent_acronym,
                    })
                    .collect())
            },
        )
        .await
    }
    
    async fn delete_queries(&self, region_id: Uuid) -> Result<(), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .delete_queries(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)?;

        // Invalidate region-level caches (queries deleted implies region reset)
        invalidate(self.infra.as_ref(), &cache_keys::region_summaries(region_id)).await;
        invalidate(self.infra.as_ref(), &cache_keys::region_status(region_id)).await;
        invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;

        Ok(())
    }

    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<ChunkSourceResponse, Self::Error> {
        let infra = &self.infra;
        cached_or_fetch(
            infra.as_ref(),
            &cache_keys::chunk_source(chunk_id),
            cache_keys::TTL_LONG,
            || async {
                // Normalize URL helper
                fn normalize_url(addr: &str) -> String {
                    if addr.starts_with("http://") || addr.starts_with("https://") {
                        addr.to_string()
                    } else {
                        let replaced = addr.replace("0.0.0.0", "localhost");
                        format!("http://{}", replaced)
                    }
                }

                let database_url = infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;

                let brainatlas_url = match infra.get_env_var("BRAINATLAS_HTTP_ADDR") {
                    Ok(addr) => normalize_url(&addr),
                    Err(_) => {
                        infra
                            .get_config(&database_url, ConfigKey::BrainatlasBaseUrl)
                            .await
                            .map_err(ServiceError::InfraError)?
                            .ok_or_else(|| ServiceError::ConfigNotFound {
                                key: "brainatlas_base_url".to_string(),
                            })?
                    }
                };

                let url = format!(
                    "{}/brainatlas-be/api/chunks/{}/source",
                    brainatlas_url.trim_end_matches('/'),
                    chunk_id
                );

                tracing::info!(url = %url, %chunk_id, "Forwarding chunk source request to brainatlas");

                infra
                    .get::<ChunkSourceResponse>(&url)
                    .await
                    .map_err(ServiceError::InfraError)
            },
        )
        .await
    }
}
