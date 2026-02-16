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
        // Get API key and model from environment
        let api_key = self.infra
            .get("OPENROUTER_API_KEY")
            .map_err(ServiceError::InfraError)?;
        let chat_model = self.infra
            .get("CHAT_MODEL")
            .unwrap_or_else(|_| "openai/gpt-4o-mini".to_string());

        self.infra
            .summarize(&api_key, &chat_model, chunks)
            .await
            .map_err(ServiceError::InfraError)
    }
    
    /// Generate search queries for a brain region
    pub async fn generate_queries(&self, region_name: &str, count: u32) -> Result<Vec<String>, ServiceError<E>> {
        // Get API key and model from environment
        let api_key = self.infra
            .get("OPENROUTER_API_KEY")
            .map_err(ServiceError::InfraError)?;
        let chat_model = self.infra
            .get("CHAT_MODEL")
            .unwrap_or_else(|_| "openai/gpt-4o-mini".to_string());

        self.infra
            .generate_queries(&api_key, &chat_model, region_name, count)
            .await
            .map_err(ServiceError::InfraError)
    }
}
