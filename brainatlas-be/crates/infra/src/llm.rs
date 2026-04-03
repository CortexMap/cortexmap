use crate::error::InfraError;
use domain::{BooleanQuery, LlmResponse, ToolCall};
use reqwest::Client;
use schemars::schema_for;
use serde::{Deserialize, Serialize};
use services::infra::{EmbeddingGenerator, LlmClient};
use std::sync::OnceLock;
use tracing::{error, info, warn};

// Load prompt template from file at compile time
fn load_prompt(name: &str) -> &'static str {
    match name {
        "summarize_rag_system" => include_str!("../prompts/summarize_rag_system.md"),
        // "generate_queries_system" => include_str!("../prompts/generate_queries_system.md"),
        // "generate_queries_user" => include_str!("../prompts/generate_queries_user.md"),
        "generate_queries_tool_system" => {
            include_str!("../prompts/generate_queries_tool_system.md")
        }
        _ => panic!("Unknown prompt: {}", name),
    }
}

// Simple template replacement function
fn render_template(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{}}}", key), value);
    }
    result
}

pub struct OpenRouterClient {
    base_url: String,
    client: OnceLock<Client>,
}

impl OpenRouterClient {
    pub fn new() -> Self {
        Self {
            base_url: "https://openrouter.ai/api/v1".to_string(),
            client: OnceLock::new(),
        }
    }

    fn get_client(&self) -> &Client {
        self.client.get_or_init(Client::new)
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
    messages: Vec<serde_json::Value>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ChatMessage {
    role: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCallResponse>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ToolCallResponse {
    id: String,
    #[serde(rename = "type")]
    call_type: Option<String>,
    function: ToolCallFunction,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ToolCallFunction {
    name: String,
    arguments: String,
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

    async fn generate_embedding(
        &self,
        api_key: &str,
        embedding_model: &str,
        text: &str,
    ) -> Result<Vec<f32>, Self::Error> {
        info!("Generating embedding for {} characters", text.len());

        let request = EmbeddingRequest {
            model: embedding_model.to_string(),
            input: text.to_string(),
        };

        let response = self
            .get_client()
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
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
            return Err(InfraError::S3(format!(
                "API error {}: {}",
                status, error_text
            )));
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

    async fn summarize_with_tools(
        &self,
        api_key: &str,
        chat_model: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<LlmResponse, Self::Error> {
        info!(
            "Calling LLM with {} messages and {} tools",
            messages.len(),
            tools.len()
        );

        let request = ChatRequest {
            model: chat_model.to_string(),
            messages: messages.to_vec(),
            temperature: 0.3,
            max_tokens: None,
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.to_vec())
            },
        };

        let response = self
            .get_client()
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                error!("Failed to call chat API: {}", e);
                InfraError::Http(e)
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("Chat API returned error {}: {}", status, error_text);
            return Err(InfraError::S3(format!(
                "API error {}: {}",
                status, error_text
            )));
        }

        let chat_response: ChatResponse = response.json().await.map_err(|e| {
            error!("Failed to parse chat response: {}", e);
            InfraError::Http(e)
        })?;

        let choice = chat_response.choices.first().ok_or_else(|| {
            error!("No choices in chat response");
            InfraError::NotFound
        })?;

        // Check if the LLM returned tool calls
        if let Some(tool_calls) = &choice.message.tool_calls
            && !tool_calls.is_empty()
        {
            let calls: Vec<ToolCall> = tool_calls
                .iter()
                .map(|tc| ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                })
                .collect();
            info!("LLM requested {} tool call(s)", calls.len());
            return Ok(LlmResponse::ToolCalls(calls));
        }

        // Otherwise, return the final text content
        let content = choice.message.content.clone().unwrap_or_default();

        info!(
            "LLM returned final response of {} characters",
            content.len()
        );
        Ok(LlmResponse::Final(content))
    }

    async fn generate_queries(
        &self,
        api_key: &str,
        chat_model: &str,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, Self::Error> {
        info!(
            "Generating {} search queries for region: {} (using tool calling)",
            count, region_name
        );

        let system_prompt = render_template(
            load_prompt("generate_queries_tool_system"),
            &[("count", &count.to_string())],
        );

        let user_prompt = format!(
            "Generate exactly {} distinct PubMed search queries for the brain region: {}. \
             Each query should target a different research aspect (anatomy, function, connectivity, disorders, development, etc.). \
             Use the create_pubmed_query tool for each query.",
            count, region_name
        );

        // Generate JSON schema for BooleanQuery using schemars
        let schema = schema_for!(BooleanQuery);
        let schema_json = serde_json::to_value(&schema).unwrap_or_else(|e| {
            error!("Failed to serialize BooleanQuery schema: {}", e);
            serde_json::json!({})
        });

        // Define the create_pubmed_query tool using the generated schema
        let tool_def = serde_json::json!({
            "type": "function",
            "function": {
                "name": "create_pubmed_query",
                "description": "Create a structured PubMed search query using BooleanQuery format for finding academic papers",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": schema_json
                    },
                    "required": ["query"]
                }
            }
        });

        let mut messages = vec![
            serde_json::json!({ "role": "system", "content": system_prompt }),
            serde_json::json!({ "role": "user", "content": user_prompt }),
        ];
        let tools = vec![tool_def];
        let mut collected_queries: Vec<String> = Vec::new();

        // Multi-turn tool calling loop (max 3 iterations)
        const MAX_ITERATIONS: usize = 3;
        for iteration in 0..MAX_ITERATIONS {
            info!(
                "Tool calling iteration {} with {} messages",
                iteration + 1,
                messages.len()
            );

            let request = ChatRequest {
                model: chat_model.to_string(),
                messages: messages.clone(),
                temperature: 0.7,
                max_tokens: None,
                tools: Some(tools.clone()),
            };

            let response = self
                .get_client()
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {}", api_key))
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
                return Err(InfraError::S3(format!(
                    "API error {}: {}",
                    status, error_text
                )));
            }

            let chat_response: ChatResponse = response.json().await.map_err(|e| {
                error!("Failed to parse chat response: {}", e);
                InfraError::Http(e)
            })?;

            let choice = chat_response.choices.first().ok_or_else(|| {
                error!("No choices in chat response");
                InfraError::NotFound
            })?;

            // Check for tool calls
            if let Some(tool_calls) = &choice.message.tool_calls
                && !tool_calls.is_empty()
            {
                // Add assistant message with tool_calls to conversation
                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": tool_calls.iter().map(|tc| serde_json::json!({
                        "id": tc.id,
                        "type": tc.call_type.as_deref().unwrap_or("function"),
                        "function": {
                            "name": tc.function.name,
                            "arguments": tc.function.arguments
                        }
                    })).collect::<Vec<_>>()
                }));

                for tc in tool_calls {
                    if tc.function.name == "create_pubmed_query" {
                        // Parse arguments as {"query": BooleanQuery}
                        match serde_json::from_str::<serde_json::Value>(&tc.function.arguments) {
                            Ok(args) => {
                                let query_json = args.get("query").cloned().unwrap_or(args.clone());

                                match serde_json::from_value::<BooleanQuery>(query_json) {
                                    Ok(bq) => {
                                        let formatted = bq.to_query_string();
                                        info!("Parsed BooleanQuery -> PubMed query: {}", formatted);
                                        collected_queries.push(formatted.clone());

                                        // Add tool response to conversation
                                        messages.push(serde_json::json!({
                                            "role": "tool",
                                            "tool_call_id": tc.id,
                                            "content": format!("Query created successfully: {}", formatted)
                                        }));
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Failed to parse BooleanQuery from tool call args: {}. \
                                             Raw args: {}. Wrapping as simple term.",
                                            e, tc.function.arguments
                                        );
                                        // Fallback: wrap the raw text as a simple term
                                        let fallback =
                                            extract_fallback_query(&tc.function.arguments);
                                        collected_queries.push(fallback.clone());

                                        messages.push(serde_json::json!({
                                            "role": "tool",
                                            "tool_call_id": tc.id,
                                            "content": format!("Query created (fallback): {}", fallback)
                                        }));
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to parse tool call arguments as JSON: {}. Raw: {}",
                                    e, tc.function.arguments
                                );
                                let fallback = extract_fallback_query(&tc.function.arguments);
                                collected_queries.push(fallback.clone());

                                messages.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": tc.id,
                                    "content": format!("Query created (fallback): {}", fallback)
                                }));
                            }
                        }
                    }
                }

                // If we've collected enough queries, stop
                if collected_queries.len() >= count as usize {
                    break;
                }
                continue;
            }

            // LLM returned text instead of tool calls — fallback to line-based parsing
            if let Some(content) = &choice.message.content
                && !content.is_empty()
            {
                warn!(
                    "LLM returned text instead of tool calls for query generation. \
                     Falling back to line-based parsing. Consider using a model that supports tool calling."
                );
                let fallback_queries = parse_text_queries(content, count);
                collected_queries.extend(fallback_queries);
                break;
            }

            // No tool calls and no content — nothing more to do
            break;
        }

        info!(
            "Generated {} queries (requested {})",
            collected_queries.len(),
            count
        );
        Ok(collected_queries)
    }
}

/// Extract a fallback query string from raw tool call arguments.
/// Tries to find any string value in the JSON, or falls back to using the raw text
/// wrapped as a BooleanQuery::term().
fn extract_fallback_query(raw_args: &str) -> String {
    // Try to extract any meaningful text from the JSON args
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw_args) {
        // Try to find a "term" or "phrase" string value recursively
        if let Some(s) = extract_first_string(&val) {
            return BooleanQuery::term(s).to_query_string();
        }
    }
    // Last resort: use the raw text cleaned up
    let cleaned = raw_args.trim().trim_matches('"').trim();
    BooleanQuery::term(cleaned).to_query_string()
}

/// Recursively extract the first string value from a JSON value
fn extract_first_string(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) => {
            for v in map.values() {
                if let Some(s) = extract_first_string(v) {
                    return Some(s);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(s) = extract_first_string(v) {
                    return Some(s);
                }
            }
            None
        }
        _ => None,
    }
}

/// Parse text-based queries (fallback when tool calling is not supported).
/// Handles numbered lists, bullet points, and plain lines.
fn parse_text_queries(text: &str, count: u32) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                // Remove numbering if present (e.g., "1. query" -> "query")
                let without_number = trimmed
                    .trim_start_matches(|c: char| {
                        c.is_numeric() || c == '.' || c == ')' || c == '-' || c == '*'
                    })
                    .trim();
                if !without_number.is_empty() {
                    // Wrap each text line as a BooleanQuery::term and format it
                    Some(BooleanQuery::term(without_number).to_query_string())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .take(count as usize)
        .collect()
}
