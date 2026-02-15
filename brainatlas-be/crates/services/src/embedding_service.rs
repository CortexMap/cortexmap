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
    pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, ServiceError<E>> {
        self.infra
            .generate_embedding(text)
            .await
            .map_err(ServiceError::InfraError)
    }
}
