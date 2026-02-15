use crate::error::InfraError;
use services::infra::{EmbeddingGenerator, LlmClient};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

pub struct OpenRouterClient {
    api_key: String,
    base_url: String,
    client: Client,
}

impl OpenRouterClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://openrouter.ai/api/v1".to_string(),
            client: Client::new(),
        }
    }
}

// Request/Response types for OpenRouter API

#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: String,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[async_trait::async_trait]
impl EmbeddingGenerator for OpenRouterClient {
    type Error = InfraError;

    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, Self::Error> {
        info!("Generating embedding for {} characters", text.len());

        let request = EmbeddingRequest {
            model: "text-embedding-3-small".to_string(),
            input: text.to_string(),
        };

        let response = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                error!("Failed to call embedding API: {}", e);
                InfraError::Http(e)
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("Embedding API returned error {}: {}", status, error_text);
            return Err(InfraError::S3(format!("API error {}: {}", status, error_text)));
        }

        let embedding_response: EmbeddingResponse = response.json().await.map_err(|e| {
            error!("Failed to parse embedding response: {}", e);
            InfraError::Http(e)
        })?;

        let embedding = embedding_response
            .data
            .first()
            .ok_or_else(|| {
                error!("No embedding data in response");
                InfraError::NotFound
            })?
            .embedding
            .clone();

        info!("Generated embedding of {} dimensions", embedding.len());
        Ok(embedding)
    }
}

#[async_trait::async_trait]
impl LlmClient for OpenRouterClient {
    type Error = InfraError;

    async fn summarize(&self, chunks: Vec<&str>) -> Result<String, Self::Error> {
        info!("Generating summary from {} chunks", chunks.len());

        // Combine all chunks with separators
        let combined_text = chunks.join("\n\n---\n\n");
        
        let system_prompt = "You are a neuroscience expert. Summarize the following research papers about a brain region. \
            Focus on: \
            1. Key anatomical features and connectivity \
            2. Primary functions and role in cognition/behavior \
            3. Clinical significance and disorders \
            4. Recent research findings. \
            Be comprehensive but concise. Use scientific terminology appropriately.";

        let user_prompt = format!("Summarize these research papers:\n\n{}", combined_text);

        let request = ChatRequest {
            model: "openai/gpt-4o-mini".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user_prompt,
                },
            ],
            temperature: 0.3,
            max_tokens: Some(2000),
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                error!("Failed to call chat API for summarization: {}", e);
                InfraError::Http(e)
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("Chat API returned error {}: {}", status, error_text);
            return Err(InfraError::S3(format!("API error {}: {}", status, error_text)));
        }

        let chat_response: ChatResponse = response.json().await.map_err(|e| {
            error!("Failed to parse chat response: {}", e);
            InfraError::Http(e)
        })?;

        let summary = chat_response
            .choices
            .first()
            .ok_or_else(|| {
                error!("No choices in chat response");
                InfraError::NotFound
            })?
            .message
            .content
            .clone();

        info!("Generated summary of {} characters", summary.len());
        Ok(summary)
    }

    async fn generate_queries(&self, region_name: &str, count: u32) -> Result<Vec<String>, Self::Error> {
        info!("Generating {} search queries for region: {}", count, region_name);

        let system_prompt = "You are a research librarian specialized in neuroscience. \
            Generate specific, targeted search queries that would find relevant academic papers about the given brain region. \
            Focus on queries that would retrieve papers about anatomy, function, connectivity, and clinical significance.";

        let user_prompt = format!(
            "Generate exactly {} distinct search queries (one per line) to find research papers about the brain region: {}. \
            Each query should target different aspects (anatomy, function, connectivity, disorders, etc.).",
            count, region_name
        );

        let request = ChatRequest {
            model: "openai/gpt-4o-mini".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user_prompt,
                },
            ],
            temperature: 0.7,
            max_tokens: Some(500),
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                error!("Failed to call chat API for query generation: {}", e);
                InfraError::Http(e)
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("Chat API returned error {}: {}", status, error_text);
            return Err(InfraError::S3(format!("API error {}: {}", status, error_text)));
        }

        let chat_response: ChatResponse = response.json().await.map_err(|e| {
            error!("Failed to parse chat response: {}", e);
            InfraError::Http(e)
        })?;

        let response_text = chat_response
            .choices
            .first()
            .ok_or_else(|| {
                error!("No choices in chat response");
                InfraError::NotFound
            })?
            .message
            .content
            .clone();

        // Parse queries from response (one per line)
        let queries: Vec<String> = response_text
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                // Remove numbering if present (e.g., "1. query" -> "query")
                if !trimmed.is_empty() {
                    let without_number = trimmed
                        .trim_start_matches(|c: char| c.is_numeric() || c == '.' || c == ')' || c == '-')
                        .trim();
                    if !without_number.is_empty() {
                        Some(without_number.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .take(count as usize) // Ensure we don't exceed requested count
            .collect();

        info!("Generated {} queries", queries.len());
        Ok(queries)
    }
}
