/// Embedding service wrapper for vector generation
use crate::{Infra, ServiceError};
use std::sync::Arc;

pub struct BrainAtlasEmbeddingService<I> {
    infra: Arc<I>,
}

impl<I> BrainAtlasEmbeddingService<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

impl<E, I> BrainAtlasEmbeddingService<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    /// Generate embedding for text
    pub async fn generate_embedding(&self, text: &str, model_override: Option<&str>) -> Result<Vec<f32>, ServiceError<E>> {
        // Get API key and model from environment
        let api_key = self.infra
            .get("OPENROUTER_API_KEY")
            .map_err(ServiceError::InfraError)?;
        let embedding_model = match model_override {
            Some(m) => m.to_string(),
            None => self.infra
                .get("EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".to_string()),
        };

        self.infra
            .generate_embedding(&api_key, &embedding_model, text)
            .await
            .map_err(ServiceError::InfraError)
    }
}
