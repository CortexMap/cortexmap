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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_embeddings_args_defaults_top_k() {
        let args: SearchEmbeddingsArgs =
            serde_json::from_str("{\"query\":\"hippocampus\"}").unwrap();

        assert_eq!(args.query, "hippocampus");
        assert_eq!(args.top_k, 5);
    }

    #[test]
    fn test_search_embeddings_args_preserves_explicit_top_k() {
        let args: SearchEmbeddingsArgs =
            serde_json::from_str("{\"query\":\"thalamus\",\"top_k\":12}").unwrap();

        assert_eq!(args.top_k, 12);
    }

    #[test]
    fn test_tool_call_serialization_round_trip() {
        let tool_call = ToolCall {
            id: "call-1".to_string(),
            name: "search_embeddings".to_string(),
            arguments: "{\"query\":\"motor cortex\",\"top_k\":3}".to_string(),
        };

        let json = serde_json::to_string(&tool_call).unwrap();
        let decoded: ToolCall = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.id, tool_call.id);
        assert_eq!(decoded.name, tool_call.name);
        assert_eq!(decoded.arguments, tool_call.arguments);
    }
}
