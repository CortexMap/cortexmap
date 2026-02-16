/// LLM service wrapper for text generation tasks
use crate::{EnvInfra, LlmClient, ServiceError};
use domain::LlmResponse;
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
    I: EnvInfra<Error = E> + LlmClient<Error = E>,
{
    /// Send a multi-turn chat with tool definitions, returning tool calls or final text
    pub async fn summarize_with_tools(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        chat_model_override: Option<&str>,
    ) -> Result<LlmResponse, ServiceError<E>> {
        let api_key = self
            .infra
            .get("OPENROUTER_API_KEY")
            .map_err(ServiceError::InfraError)?;
        let chat_model = match chat_model_override {
            Some(m) => m.to_string(),
            None => self
                .infra
                .get("CHAT_MODEL")
                .unwrap_or_else(|_| "openai/gpt-4o-mini".to_string()),
        };

        self.infra
            .summarize_with_tools(&api_key, &chat_model, messages, tools)
            .await
            .map_err(ServiceError::InfraError)
    }

    /// Generate search queries for a brain region
    pub async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, ServiceError<E>> {
        // Get API key and model from environment
        let api_key = self
            .infra
            .get("OPENROUTER_API_KEY")
            .map_err(ServiceError::InfraError)?;
        let chat_model = self
            .infra
            .get("CHAT_MODEL")
            .unwrap_or_else(|_| "openai/gpt-4o-mini".to_string());

        self.infra
            .generate_queries(&api_key, &chat_model, region_name, count)
            .await
            .map_err(ServiceError::InfraError)
    }
}
