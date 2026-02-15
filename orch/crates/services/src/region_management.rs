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
    I: EnvInfra<Error = E> + HttpClient<Error = E> + BatchManagement<Error = E> + crate::OrchDatabase<Error = E> + Send + Sync,
{
    type Error = ServiceError<E>;

    async fn get_summaries(&self, region_id: i32) -> Result<Vec<RegionSummary>, Self::Error> {
        // Try env var first, fall back to config
        let brainatlas_url = match self.infra.get_env_var("BRAINATLAS_URL") {
            Ok(url) => url,
            Err(_) => {
                let database_url = self
                    .infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;
                
                self.infra
                    .get_config(&database_url, ConfigKey::BrainatlasBaseUrl)
                    .await
                    .map_err(ServiceError::InfraError)?
                    .ok_or_else(|| ServiceError::ConfigNotFound {
                        key: "brainatlas_base_url".to_string(),
                    })?
            }
        };

        let url = format!("{}/brainatlas-be/api/search", brainatlas_url.trim_end_matches('/'));

        // TODO: Call brainatlas API to get summaries
        // For now, return empty since brainatlas doesn't have a "get summaries by region_id" endpoint yet
        // We'll need to add this to brainatlas
        tracing::warn!(region_id, "get_summaries not yet implemented in brainatlas");
        Ok(vec![])
    }

    async fn get_active_batch(
        &self,
        region_id: i32,
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

    async fn get_queries(&self, region_id: i32) -> Result<Vec<RegionQuery>, Self::Error> {
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
        region_id: i32,
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
        // TODO: Call LLM to generate queries
        // For now, generate simple queries
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
}
