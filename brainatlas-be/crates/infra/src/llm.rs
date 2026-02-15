use crate::error::InfraError;
use services::infra::{EmbeddingGenerator, LlmClient};

pub struct OpenRouterClient {
    api_key: String,
    base_url: String,
    // TODO: Add reqwest::Client
}

impl OpenRouterClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://openrouter.ai/api/v1".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingGenerator for OpenRouterClient {
    type Error = InfraError;

    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, Self::Error> {
        // TODO: Implement embedding generation
        // 1. POST to {base_url}/embeddings
        // 2. Body: { "model": "text-embedding-3-small", "input": text }
        // 3. Headers: { "Authorization": "Bearer {api_key}" }
        // 4. Parse response, extract embedding vector
        
        tracing::warn!("EmbeddingGenerator::generate_embedding not yet implemented - returning empty");
        Err(InfraError::NotImplemented)
    }
}

#[async_trait::async_trait]
impl LlmClient for OpenRouterClient {
    type Error = InfraError;

    async fn summarize(&self, chunks: Vec<&str>) -> Result<String, Self::Error> {
        // TODO: Implement summarization
        // 1. Combine chunks intelligently (may need to chunk if too long)
        // 2. POST to {base_url}/chat/completions
        // 3. Model: "openai/gpt-4o-mini" or similar
        // 4. System prompt: "You are a neuroscience expert..."
        // 5. User prompt: chunks joined
        // 6. Parse response, extract summary text
        
        tracing::warn!("LlmClient::summarize not yet implemented - returning placeholder");
        Err(InfraError::NotImplemented)
    }

    async fn generate_queries(&self, region_name: &str, count: usize) -> Result<Vec<String>, Self::Error> {
        // TODO: Implement query generation
        // 1. POST to {base_url}/chat/completions
        // 2. Prompt: "Generate {count} search queries to find research papers about brain region: {region_name}"
        // 3. Parse response as JSON array of queries
        
        tracing::warn!("LlmClient::generate_queries not yet implemented - returning placeholder");
        Err(InfraError::NotImplemented)
    }
}
