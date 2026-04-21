//! LLM token-usage value types shared across domain, services, infra and rpc layers.
//!
//! These are pure data types with no dependencies on a specific provider. They
//! are produced at the infra boundary (where the provider response is parsed),
//! accounted for in the services layer, and persisted via the `LlmUsageRepo`
//! port.

use serde::{Deserialize, Serialize};

/// Token usage counts returned by an LLM provider.
///
/// The three fields should satisfy `total_tokens == prompt_tokens + completion_tokens`
/// for chat completions, and `total_tokens == prompt_tokens` (with
/// `completion_tokens == 0`) for embeddings. We do not enforce this invariant in
/// code because some providers diverge slightly; we simply record what the
/// provider returned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl Usage {
    pub fn new(prompt_tokens: u32, completion_tokens: u32, total_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        }
    }

    /// Element-wise add two `Usage` records (used when aggregating
    /// multi-iteration calls like `generate_queries`).
    pub fn saturating_add(self, other: Usage) -> Usage {
        Usage {
            prompt_tokens: self.prompt_tokens.saturating_add(other.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_add(other.completion_tokens),
            total_tokens: self.total_tokens.saturating_add(other.total_tokens),
        }
    }
}

/// Classifies which OpenRouter endpoint produced an `LlmCallOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmEndpointKind {
    Embedding,
    ChatCompletion,
    ChatCompletionWithTools,
}

impl LlmEndpointKind {
    /// Short tag persisted in the `endpoint` column of `llm_call_usage`.
    pub fn as_tag(self) -> &'static str {
        match self {
            LlmEndpointKind::Embedding => "embedding",
            LlmEndpointKind::ChatCompletion => "chat",
            LlmEndpointKind::ChatCompletionWithTools => "chat_tools",
        }
    }
}

/// Wrapper that lets an LLM trait method return both the business payload and
/// the usage metadata captured from the provider response in a single value.
#[derive(Debug, Clone)]
pub struct LlmCallOutcome<T> {
    pub value: T,
    pub usage: Usage,
    pub model: String,
    pub endpoint: LlmEndpointKind,
}

impl<T> LlmCallOutcome<T> {
    pub fn new(value: T, usage: Usage, model: String, endpoint: LlmEndpointKind) -> Self {
        Self {
            value,
            usage,
            model,
            endpoint,
        }
    }

    /// Consume the outcome and return the underlying value, discarding usage.
    /// Useful for call sites that haven't been wired up to accounting yet.
    pub fn into_value(self) -> T {
        self.value
    }

    /// Map the inner value while preserving metadata.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> LlmCallOutcome<U> {
        LlmCallOutcome {
            value: f(self.value),
            usage: self.usage,
            model: self.model,
            endpoint: self.endpoint,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_saturating_add_sums_fields() {
        let a = Usage::new(10, 20, 30);
        let b = Usage::new(5, 7, 12);
        let c = a.saturating_add(b);
        assert_eq!(c.prompt_tokens, 15);
        assert_eq!(c.completion_tokens, 27);
        assert_eq!(c.total_tokens, 42);
    }

    #[test]
    fn endpoint_kind_tags_are_stable() {
        assert_eq!(LlmEndpointKind::Embedding.as_tag(), "embedding");
        assert_eq!(LlmEndpointKind::ChatCompletion.as_tag(), "chat");
        assert_eq!(
            LlmEndpointKind::ChatCompletionWithTools.as_tag(),
            "chat_tools"
        );
    }

    #[test]
    fn outcome_map_preserves_metadata() {
        let o = LlmCallOutcome::new(
            42u32,
            Usage::new(1, 2, 3),
            "openai/gpt-4o-mini".to_string(),
            LlmEndpointKind::ChatCompletion,
        );
        let mapped = o.map(|v| v.to_string());
        assert_eq!(mapped.value, "42");
        assert_eq!(mapped.usage, Usage::new(1, 2, 3));
        assert_eq!(mapped.model, "openai/gpt-4o-mini");
        assert_eq!(mapped.endpoint, LlmEndpointKind::ChatCompletion);
    }
}
