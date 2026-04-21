/// Embedding service wrapper for vector generation.
///
/// Like `BrainAtlasLlmService`, this owns a `CostAccountant` and records a
/// `llm_call_usage` row for every successful embedding call.
use crate::cost_accounting::CostAccountant;
use crate::{Infra, ServiceError};
use domain::UsageContext;
use std::sync::Arc;
use std::time::Instant;

pub struct BrainAtlasEmbeddingService<I> {
    infra: Arc<I>,
    accountant: CostAccountant<I>,
}

impl<I> BrainAtlasEmbeddingService<I> {
    pub fn new(infra: Arc<I>) -> Self {
        let accountant = CostAccountant::new(infra.clone());
        Self { infra, accountant }
    }
}

impl<E, I> BrainAtlasEmbeddingService<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E> + 'static,
{
    /// Generate embedding for text.
    pub async fn generate_embedding(
        &self,
        text: &str,
        model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<Vec<f32>, ServiceError<E>> {
        let api_key = self
            .infra
            .get("OPENROUTER_API_KEY")
            .map_err(ServiceError::InfraError)?;
        let embedding_model = match model_override {
            Some(m) => m.to_string(),
            None => self
                .infra
                .get("EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".to_string()),
        };

        let started = Instant::now();
        let ctx = ctx.with_caller_tag("embed");
        let outcome = self
            .infra
            .generate_embedding(&api_key, &embedding_model, text)
            .await
            .map_err(ServiceError::InfraError);
        self.accountant.finish(outcome, ctx, started).await
    }
}
