use crate::{BatchManagement, EnvInfra, HttpClient, ServiceError};
use app::RegionManagement;
use domain::{BatchStatus, ConfigKey, ProcessingBatch, RegionQuery, RegionSummary};
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
    I: EnvInfra<Error = E> + HttpClient<Error = E> + BatchManagement<Error = E> + crate::OrchDatabase<Error = E> + crate::RegionMappingQueries<Error = E> + Send + Sync,
{
    type Error = ServiceError<E>;

    async fn get_summaries(&self, region_id: Uuid) -> Result<Vec<RegionSummary>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        // First get the region mapping to find the Int4 region_id
        let region_mapping = self
            .infra
            .get_region_mapping(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)?
            .ok_or_else(|| ServiceError::NotFound)?;

        // Query the region_summary table directly
        let summaries = self
            .infra
            .get_region_summaries(&database_url, region_mapping.region_id)
            .await
            .map_err(ServiceError::InfraError)?;

        // Convert to domain::RegionSummary
        Ok(summaries
            .into_iter()
            .map(|s| RegionSummary {
                summary: s.summary.unwrap_or_default(),
                created_at: chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    s.created_at,
                    chrono::Utc,
                ),
            })
            .collect())
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
            .map_err(ServiceError::InfraError)
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
        // For now, generate simple queries until LLM integration is added
        tracing::info!(region_name, count, "Generating queries for region");
        
        // Placeholder queries until LLM integration is added
        Ok(vec![
            format!("{} research papers", region_name),
            format!("{} neuroscience studies", region_name),
            format!("{} brain function", region_name),
        ][..count.min(3) as usize].to_vec())
    }
    
    async fn get_batches_by_status(
        &self,
        status: BatchStatus,
    ) -> Result<Vec<ProcessingBatch>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_batches_by_status(&database_url, status)
            .await
            .map_err(ServiceError::InfraError)
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
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let regions = self
            .infra
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
    }
}
