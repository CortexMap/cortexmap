use crate::error::InfraError;
use domain::{BooleanQuery, LlmCallOutcome, LlmEndpointKind, LlmResponse, ToolCall, Usage};
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

/// Build a structured "region identity" block that is substituted into the
/// `generate_queries_tool_system` prompt. Gives the LLM a single, easy-to-read
/// reference card with the region's full name, its acronym, and parent
/// context — so it can pick literature-targeted OR alternatives instead of
/// hallucinating synonyms from sub-modifier tokens like "ventral part".
fn render_region_identity_context(
    region_name: &str,
    acronym: Option<&str>,
    parent_name: Option<&str>,
    parent_acronym: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!("- Full region name: {}", region_name));
    match acronym {
        Some(a) if !a.trim().is_empty() => {
            lines.push(format!("- Region acronym: {}", a))
        }
        _ => lines
            .push("- Region acronym: (none in source ontology)".to_string()),
    }
    match parent_name {
        Some(p) if !p.trim().is_empty() => {
            lines.push(format!("- Parent region name: {}", p))
        }
        _ => lines.push("- Parent region name: (root region)".to_string()),
    }
    match parent_acronym {
        Some(pa) if !pa.trim().is_empty() => {
            lines.push(format!("- Parent region acronym: {}", pa))
        }
        _ => lines
            .push("- Parent region acronym: (root region)".to_string()),
    }
    lines.join("\n")
}

/// Stateless OpenAI-compatible HTTP client shared by OpenRouter and Requesty.
///
/// The base URL is NOT stored on the client: it's threaded through the
/// `EmbeddingGenerator` and `LlmClient` trait methods per call, so the same
/// instance can serve different providers across requests.
pub struct OpenAiCompatibleClient {
    client: OnceLock<Client>,
}

impl Default for OpenAiCompatibleClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAiCompatibleClient {
    pub fn new() -> Self {
        Self {
            client: OnceLock::new(),
        }
    }

    fn get_client(&self) -> &Client {
        self.client.get_or_init(Client::new)
    }
}

/// Back-compat alias. Kept for one release cycle so downstream docs / tests
/// that still reference `OpenRouterClient` keep compiling. Prefer the new
/// name in fresh code.
#[allow(dead_code)]
pub type OpenRouterClient = OpenAiCompatibleClient;

// Request/Response types for OpenAI-compatible APIs (OpenRouter, Requesty, …)

#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: String,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
    #[serde(default)]
    usage: Option<UsageWire>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

/// Token usage block returned by OpenAI-compatible gateways. Only the three
/// fields we care about are deserialized; any extras are ignored.
#[derive(Deserialize, Default, Debug, Clone, Copy)]
struct UsageWire {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

impl From<UsageWire> for Usage {
    fn from(w: UsageWire) -> Self {
        Usage {
            prompt_tokens: w.prompt_tokens,
            completion_tokens: w.completion_tokens,
            total_tokens: w.total_tokens,
        }
    }
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
    #[serde(default)]
    usage: Option<UsageWire>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[async_trait::async_trait]
impl EmbeddingGenerator for OpenAiCompatibleClient {
    type Error = InfraError;

    async fn generate_embedding(
        &self,
        base_url: &str,
        api_key: &str,
        embedding_model: &str,
        text: &str,
    ) -> Result<LlmCallOutcome<Vec<f32>>, Self::Error> {
        info!("Generating embedding for {} characters", text.len());

        let request = EmbeddingRequest {
            model: embedding_model.to_string(),
            input: text.to_string(),
        };

        let response = self
            .get_client()
            .post(format!("{}/embeddings", base_url))
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

        let usage: Usage = match embedding_response.usage {
            Some(u) => u.into(),
            None => {
                warn!(
                    model = embedding_model,
                    "Embedding response omitted `usage` block; cost tracking will record zero tokens"
                );
                Usage::default()
            }
        };

        let embedding = embedding_response
            .data
            .first()
            .ok_or_else(|| {
                error!("No embedding data in response");
                InfraError::NotFound
            })?
            .embedding
            .clone();

        info!(
            "Generated embedding of {} dimensions (prompt_tokens={}, total_tokens={})",
            embedding.len(),
            usage.prompt_tokens,
            usage.total_tokens
        );
        Ok(LlmCallOutcome::new(
            embedding,
            usage,
            embedding_model.to_string(),
            LlmEndpointKind::Embedding,
        ))
    }
}

#[async_trait::async_trait]
impl LlmClient for OpenAiCompatibleClient {
    type Error = InfraError;

    async fn summarize_with_tools(
        &self,
        base_url: &str,
        api_key: &str,
        chat_model: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<LlmCallOutcome<LlmResponse>, Self::Error> {
        info!(
            "Calling LLM with {} messages and {} tools",
            messages.len(),
            tools.len()
        );

        let has_tools = !tools.is_empty();
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
            .post(format!("{}/chat/completions", base_url))
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

        let usage: Usage = match chat_response.usage {
            Some(u) => u.into(),
            None => {
                warn!(
                    model = chat_model,
                    "Chat response omitted `usage` block; cost tracking will record zero tokens"
                );
                Usage::default()
            }
        };

        let choice = chat_response.choices.first().ok_or_else(|| {
            error!("No choices in chat response");
            InfraError::NotFound
        })?;

        let endpoint = if has_tools {
            LlmEndpointKind::ChatCompletionWithTools
        } else {
            LlmEndpointKind::ChatCompletion
        };

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
            return Ok(LlmCallOutcome::new(
                LlmResponse::ToolCalls(calls),
                usage,
                chat_model.to_string(),
                endpoint,
            ));
        }

        // Otherwise, return the final text content
        let content = choice.message.content.clone().unwrap_or_default();

        info!(
            "LLM returned final response of {} characters (prompt_tokens={}, completion_tokens={})",
            content.len(),
            usage.prompt_tokens,
            usage.completion_tokens,
        );
        Ok(LlmCallOutcome::new(
            LlmResponse::Final(content),
            usage,
            chat_model.to_string(),
            endpoint,
        ))
    }

    async fn generate_queries(
        &self,
        base_url: &str,
        api_key: &str,
        chat_model: &str,
        region_name: &str,
        count: u32,
        acronym: Option<&str>,
        parent_name: Option<&str>,
        parent_acronym: Option<&str>,
    ) -> Result<LlmCallOutcome<Vec<String>>, Self::Error> {
        info!(
            "Generating {} search queries for region: {} (acronym={:?}, parent={:?}/{:?}) (using tool calling)",
            count, region_name, acronym, parent_name, parent_acronym
        );

        // Build a structured identity context block. The prompt template
        // substitutes {{REGION_CONTEXT_BLOCK}} with this so the LLM has
        // every anchor it needs to construct on-target OR groups.
        let region_context_block = render_region_identity_context(
            region_name,
            acronym,
            parent_name,
            parent_acronym,
        );

        let system_prompt = render_template(
            load_prompt("generate_queries_tool_system"),
            &[
                ("count", &count.to_string()),
                ("REGION_NAME", region_name),
                ("REGION_ACRONYM", acronym.unwrap_or("(unknown)")),
                ("PARENT_NAME", parent_name.unwrap_or("(unknown)")),
                ("PARENT_ACRONYM", parent_acronym.unwrap_or("(unknown)")),
                ("REGION_CONTEXT_BLOCK", &region_context_block),
            ],
        );

        let user_prompt = format!(
            "Generate exactly {} distinct PubMed search queries for the brain region: {}{}. \
             Each query should target a different research aspect (anatomy, function, connectivity, disorders, development, etc.). \
             Use the create_pubmed_query tool for each query.",
            count,
            region_name,
            acronym.map(|a| format!(" (acronym: {})", a)).unwrap_or_default(),
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
        let mut aggregated_usage = Usage::default();

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
                .post(format!("{}/chat/completions", base_url))
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

            // Accumulate usage across iterations.
            match chat_response.usage {
                Some(u) => {
                    aggregated_usage = aggregated_usage.saturating_add(u.into());
                }
                None => {
                    warn!(
                        model = chat_model,
                        iteration = iteration + 1,
                        "Query-gen chat response omitted `usage`; treating iteration as 0 tokens"
                    );
                }
            }

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
            "Generated {} queries (requested {}, aggregated prompt_tokens={}, completion_tokens={})",
            collected_queries.len(),
            count,
            aggregated_usage.prompt_tokens,
            aggregated_usage.completion_tokens,
        );
        Ok(LlmCallOutcome::new(
            collected_queries,
            aggregated_usage,
            chat_model.to_string(),
            LlmEndpointKind::ChatCompletionWithTools,
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template_replaces_multiple_occurrences() {
        let rendered = render_template(
            "Generate {count} queries for {region}. {region} should be specific.",
            &[("count", "3"), ("region", "hippocampus")],
        );

        assert_eq!(
            rendered,
            "Generate 3 queries for hippocampus. hippocampus should be specific."
        );
    }

    #[test]
    fn test_extract_first_string_finds_nested_values() {
        let value = serde_json::json!({
            "query": {
                "or": [1, {"field": {"value": "motor cortex"}}]
            }
        });

        assert_eq!(
            extract_first_string(&value),
            Some("motor cortex".to_string())
        );
    }

    #[test]
    fn test_extract_first_string_returns_none_when_no_strings_exist() {
        let value = serde_json::json!({"items": [1, true, null, {"value": 4}]});
        assert_eq!(extract_first_string(&value), None);
    }

    #[test]
    fn test_extract_fallback_query_uses_nested_json_string_value() {
        let raw_args = "{\"query\":{\"field\":{\"value\":\"motor cortex\"}}}";

        assert_eq!(
            extract_fallback_query(raw_args),
            BooleanQuery::term("motor cortex").to_query_string()
        );
    }

    #[test]
    fn test_extract_fallback_query_trims_raw_text_when_json_is_invalid() {
        assert_eq!(
            extract_fallback_query("  \"basal ganglia\"  "),
            BooleanQuery::term("basal ganglia").to_query_string()
        );
    }

    #[test]
    fn test_parse_text_queries_handles_lists_and_limits_count() {
        let text = "1. motor cortex\n- hippocampus\n* thalamus\n\n4) cerebellum";

        let queries = parse_text_queries(text, 3);

        assert_eq!(
            queries,
            vec![
                BooleanQuery::term("motor cortex").to_query_string(),
                BooleanQuery::term("hippocampus").to_query_string(),
                BooleanQuery::term("thalamus").to_query_string(),
            ]
        );
    }

    // ── Usage-block parsing tests (Task 21 in plan) ──────────────────────

    #[test]
    fn test_chat_response_parses_usage_when_present() {
        let json = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "hi" } }],
            "usage": { "prompt_tokens": 12, "completion_tokens": 7, "total_tokens": 19 }
        });
        let parsed: ChatResponse = serde_json::from_value(json).expect("ChatResponse decodes");
        let usage: Usage = parsed.usage.expect("usage present").into();
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 7);
        assert_eq!(usage.total_tokens, 19);
    }

    #[test]
    fn test_chat_response_missing_usage_is_none() {
        let json = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "hi" } }]
        });
        let parsed: ChatResponse = serde_json::from_value(json).expect("decodes without usage");
        assert!(parsed.usage.is_none());
    }

    #[test]
    fn test_chat_response_partial_usage_defaults_missing_fields_to_zero() {
        // OpenRouter occasionally sends only `total_tokens` (e.g. streaming
        // finalization). We should still decode cleanly and zero-fill.
        let json = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "hi" } }],
            "usage": { "total_tokens": 5 }
        });
        let parsed: ChatResponse = serde_json::from_value(json).expect("decodes partial usage");
        let usage: Usage = parsed.usage.expect("usage present").into();
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 0);
        assert_eq!(usage.total_tokens, 5);
    }

    #[test]
    fn test_embedding_response_parses_usage_when_present() {
        let json = serde_json::json!({
            "data": [{ "embedding": [0.1, 0.2, 0.3] }],
            "usage": { "prompt_tokens": 42, "total_tokens": 42 }
        });
        let parsed: EmbeddingResponse =
            serde_json::from_value(json).expect("EmbeddingResponse decodes");
        let usage: Usage = parsed.usage.expect("usage present").into();
        assert_eq!(usage.prompt_tokens, 42);
        assert_eq!(usage.total_tokens, 42);
    }

    #[test]
    fn test_embedding_response_missing_usage_is_none() {
        let json = serde_json::json!({
            "data": [{ "embedding": [0.1, 0.2, 0.3] }]
        });
        let parsed: EmbeddingResponse =
            serde_json::from_value(json).expect("decodes without usage");
        assert!(parsed.usage.is_none());
    }

    // ── Gap-fill tests (Plan Task 1.10) ──────────────────────────────────

    /// Tool-calling loop in `OpenAiCompatibleClient::generate_queries` aggregates
    /// `usage` across iterations via `Usage::saturating_add` (see
    /// `llm.rs:437-449`). Simulate two iterations by parsing two successive
    /// `ChatResponse` wire payloads and summing their usage — the resulting
    /// `Usage` must be the element-wise total.
    #[test]
    fn test_tool_calling_loop_aggregates_usage_across_iterations() {
        let iter1 = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "i1" } }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 40, "total_tokens": 140 }
        });
        let iter2 = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "i2" } }],
            "usage": { "prompt_tokens": 25, "completion_tokens": 10, "total_tokens": 35 }
        });

        let r1: ChatResponse = serde_json::from_value(iter1).unwrap();
        let r2: ChatResponse = serde_json::from_value(iter2).unwrap();

        // Mirror the production aggregation loop exactly.
        let mut aggregated = Usage::default();
        for r in [r1, r2] {
            match r.usage {
                Some(u) => aggregated = aggregated.saturating_add(u.into()),
                None => { /* treat as zero-token iteration */ }
            }
        }

        assert_eq!(aggregated.prompt_tokens, 125);
        assert_eq!(aggregated.completion_tokens, 50);
        assert_eq!(aggregated.total_tokens, 175);
    }

    /// Aggregation must survive an iteration where the upstream response
    /// omitted the `usage` block entirely — that iteration contributes zero.
    #[test]
    fn test_tool_calling_loop_skips_iterations_without_usage() {
        let iter1 = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "i1" } }],
            "usage": { "prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10 }
        });
        let iter2 = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "i2" } }]
        });

        let r1: ChatResponse = serde_json::from_value(iter1).unwrap();
        let r2: ChatResponse = serde_json::from_value(iter2).unwrap();
        assert!(r2.usage.is_none());

        let mut aggregated = Usage::default();
        for r in [r1, r2] {
            if let Some(u) = r.usage {
                aggregated = aggregated.saturating_add(u.into());
            }
        }

        assert_eq!(aggregated.prompt_tokens, 7);
        assert_eq!(aggregated.completion_tokens, 3);
        assert_eq!(aggregated.total_tokens, 10);
    }

    /// An explicitly empty usage block (`"usage": {}`) must decode with every
    /// field defaulted to zero — this mirrors the worst-case provider
    /// response where a `usage` key is present but empty.
    #[test]
    fn test_chat_response_empty_usage_object_zero_fills_all_fields() {
        let json = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "hi" } }],
            "usage": {}
        });
        let parsed: ChatResponse = serde_json::from_value(json).expect("empty usage decodes");
        let usage: Usage = parsed.usage.expect("usage present").into();
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 0);
        assert_eq!(usage.total_tokens, 0);
    }

    /// When `completion_tokens` is omitted (e.g. some providers only report
    /// `prompt_tokens` + `total_tokens`), the missing field defaults to zero
    /// without failing deserialisation.
    #[test]
    fn test_chat_response_missing_completion_tokens_defaults_to_zero() {
        let json = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "hi" } }],
            "usage": { "prompt_tokens": 8, "total_tokens": 8 }
        });
        let parsed: ChatResponse = serde_json::from_value(json).expect("decodes partial usage");
        let usage: Usage = parsed.usage.expect("usage present").into();
        assert_eq!(usage.prompt_tokens, 8);
        assert_eq!(usage.completion_tokens, 0);
        assert_eq!(usage.total_tokens, 8);
    }

    /// `LlmCallOutcome::new` round-trips every field — this is the outcome
    /// produced by `summarize_with_tools` on both the final-text and
    /// tool-call branches of `llm.rs:312-334`.
    #[test]
    fn test_llm_call_outcome_new_preserves_fields_for_missing_usage_fallback() {
        // Production code at llm.rs:277-286 substitutes `Usage::default()`
        // when the response omits `usage`. Construct the resulting outcome
        // directly and assert the contract.
        let outcome = LlmCallOutcome::new(
            domain::LlmResponse::Final("text".to_string()),
            Usage::default(),
            "openai/gpt-4o-mini".to_string(),
            LlmEndpointKind::ChatCompletion,
        );
        assert_eq!(outcome.usage.prompt_tokens, 0);
        assert_eq!(outcome.usage.completion_tokens, 0);
        assert_eq!(outcome.usage.total_tokens, 0);
        assert_eq!(outcome.model, "openai/gpt-4o-mini");
        match outcome.value {
            domain::LlmResponse::Final(t) => assert_eq!(t, "text"),
            _ => panic!("expected Final"),
        }
    }

    /// `UsageWire` ignores unknown fields (serde's default) — providers
    /// occasionally send extras like `prompt_tokens_details`. We must not
    /// fail deserialisation on them.
    #[test]
    fn test_chat_response_usage_tolerates_unknown_fields() {
        let json = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "hi" } }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 4,
                "total_tokens": 15,
                "prompt_tokens_details": { "cached_tokens": 0 },
                "completion_tokens_details": { "reasoning_tokens": 7 }
            }
        });
        let parsed: ChatResponse =
            serde_json::from_value(json).expect("decodes despite extra fields");
        let usage: Usage = parsed.usage.expect("usage present").into();
        assert_eq!(usage.prompt_tokens, 11);
        assert_eq!(usage.completion_tokens, 4);
        assert_eq!(usage.total_tokens, 15);
    }
}
