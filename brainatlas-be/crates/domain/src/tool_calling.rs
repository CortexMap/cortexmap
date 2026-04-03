use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A tool call request from the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// A tool result to send back to the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
}

/// Parsed arguments for the search_embeddings tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchEmbeddingsArgs {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

fn default_top_k() -> usize {
    5
}

/// Response from an LLM call that may include tool calls
#[derive(Debug, Clone)]
pub enum LlmResponse {
    /// The LLM wants to call one or more tools
    ToolCalls(Vec<ToolCall>),
    /// The LLM produced a final text response
    Final(String),
}
