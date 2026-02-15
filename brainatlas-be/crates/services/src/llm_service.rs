/// LLM service wrapper for text generation tasks
use crate::{Infra, ServiceError};
use std::sync::Arc;

pub struct BrainAtlasLlmService<I> {
    infra: Arc<I>,
}

impl<I> BrainAtlasLlmService<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

impl<E, I> BrainAtlasLlmService<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    /// Generate summary from text chunks
    pub async fn summarize(&self, chunks: Vec<&str>) -> Result<String, ServiceError<E>> {
        self.infra
            .summarize(chunks)
            .await
            .map_err(ServiceError::InfraError)
    }
    
    /// Generate search queries for a brain region
    pub async fn generate_queries(&self, region_name: &str, count: u32) -> Result<Vec<String>, ServiceError<E>> {
        self.infra
            .generate_queries(region_name, count)
            .await
            .map_err(ServiceError::InfraError)
    }
}
