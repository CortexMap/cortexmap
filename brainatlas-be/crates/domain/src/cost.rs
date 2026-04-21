//! Domain types for LLM cost tracking: pricing catalogue entries, the record
//! persisted for every outbound LLM call, and aggregate views.

use crate::{LlmEndpointKind, Usage};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Pricing for a single model. Prices are denominated in USD per **million**
/// tokens. `embedding_price_per_million` is only populated for embedding
/// models; for chat models it is `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmPricing {
    pub model: String,
    pub input_price_per_million: f64,
    pub output_price_per_million: f64,
    pub embedding_price_per_million: Option<f64>,
    pub currency: String,
    pub effective_from: DateTime<Utc>,
}

impl LlmPricing {
    /// Compute the cost in USD for a single observed `Usage` against this
    /// pricing row. Returns `None` if the pricing is missing an essential
    /// component (e.g. embedding call against a chat-only row).
    ///
    /// The formula is
    ///
    /// - chat / chat_tools:
    ///   `(prompt_tokens * input_price + completion_tokens * output_price) / 1_000_000`
    /// - embedding:
    ///   `total_tokens * embedding_price / 1_000_000` if the pricing row has an
    ///   embedding price, otherwise `total_tokens * input_price / 1_000_000`
    pub fn compute_cost_usd(&self, usage: Usage, endpoint: LlmEndpointKind) -> f64 {
        match endpoint {
            LlmEndpointKind::Embedding => {
                let price = self
                    .embedding_price_per_million
                    .unwrap_or(self.input_price_per_million);
                (usage.total_tokens as f64) * price / 1_000_000.0
            }
            LlmEndpointKind::ChatCompletion | LlmEndpointKind::ChatCompletionWithTools => {
                ((usage.prompt_tokens as f64) * self.input_price_per_million
                    + (usage.completion_tokens as f64) * self.output_price_per_million)
                    / 1_000_000.0
            }
        }
    }
}

/// Insert model for the `llm_call_usage` table. One row per logical LLM call
/// (multi-iteration calls like `generate_queries` aggregate into one row).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewLlmCallUsage {
    pub endpoint: String,
    pub model: String,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub total_tokens: i32,
    pub cost_usd: Option<f64>,
    pub correlation_id: Option<String>,
    pub region_id: Option<i32>,
    pub summary_id: Option<Uuid>,
    pub batch_id: Option<Uuid>,
    pub caller_tag: Option<String>,
    pub request_id: Option<String>,
}

/// Filter for the aggregate query against `llm_call_usage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageAggregateFilter {
    /// Inclusive lower-bound on `created_at`.
    pub since: Option<DateTime<Utc>>,
    /// Inclusive upper-bound on `created_at`.
    pub until: Option<DateTime<Utc>>,
    pub model: Option<String>,
    pub correlation_id: Option<String>,
    /// When set, matches rows where `correlation_id` starts with this prefix.
    /// Useful for aggregating all LLM calls under an eval run via
    /// `eval:{run_id}:` regardless of step id.
    pub correlation_id_prefix: Option<String>,
    pub region_id: Option<i32>,
    pub summary_id: Option<Uuid>,
    pub batch_id: Option<Uuid>,
    pub caller_tag: Option<String>,
}

/// Aggregate view of `llm_call_usage` matching a `UsageAggregateFilter`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageAggregate {
    pub total_cost_usd: f64,
    pub total_tokens: i64,
    pub total_prompt_tokens: i64,
    pub total_completion_tokens: i64,
    pub total_calls: i64,
    pub by_model: Vec<UsageByModel>,
    pub by_caller_tag: Vec<UsageByCallerTag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageByModel {
    pub model: String,
    pub total_cost_usd: f64,
    pub total_tokens: i64,
    pub total_calls: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageByCallerTag {
    pub caller_tag: String,
    pub total_cost_usd: f64,
    pub total_tokens: i64,
    pub total_calls: i64,
}

/// Context threaded into the cost accounting helper alongside an
/// `LlmCallOutcome<T>`. All fields except `caller_tag` are optional: they let
/// the persisted `llm_call_usage` row link back to the originating region,
/// batch or summary. When unknown, the caller leaves them `None`.
#[derive(Debug, Clone, Default)]
pub struct UsageContext {
    pub correlation_id: Option<String>,
    pub region_id: Option<i32>,
    pub summary_id: Option<Uuid>,
    pub batch_id: Option<Uuid>,
    pub caller_tag: Option<String>,
    pub request_id: Option<String>,
}

impl UsageContext {
    pub fn with_caller_tag(mut self, tag: &str) -> Self {
        self.caller_tag = Some(tag.to_string());
        self
    }

    pub fn with_correlation(mut self, correlation_id: Option<String>) -> Self {
        self.correlation_id = correlation_id;
        self
    }

    pub fn with_region(mut self, region_id: Option<i32>) -> Self {
        self.region_id = region_id;
        self
    }

    pub fn with_summary(mut self, summary_id: Option<Uuid>) -> Self {
        self.summary_id = summary_id;
        self
    }

    pub fn with_batch(mut self, batch_id: Option<Uuid>) -> Self {
        self.batch_id = batch_id;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pricing() -> LlmPricing {
        LlmPricing {
            model: "openai/gpt-4o-mini".to_string(),
            input_price_per_million: 0.15,
            output_price_per_million: 0.60,
            embedding_price_per_million: None,
            currency: "USD".to_string(),
            effective_from: Utc::now(),
        }
    }

    #[test]
    fn chat_cost_uses_input_and_output_prices() {
        let p = pricing();
        // 1_000_000 prompt tokens at $0.15 + 1_000_000 completion tokens at $0.60
        // = $0.75
        let usage = Usage::new(1_000_000, 1_000_000, 2_000_000);
        let cost = p.compute_cost_usd(usage, LlmEndpointKind::ChatCompletion);
        assert!((cost - 0.75).abs() < 1e-9);
    }

    #[test]
    fn embedding_cost_falls_back_to_input_price_when_unset() {
        let mut p = pricing();
        p.embedding_price_per_million = None;
        let usage = Usage::new(2_000_000, 0, 2_000_000);
        let cost = p.compute_cost_usd(usage, LlmEndpointKind::Embedding);
        assert!((cost - 0.30).abs() < 1e-9);
    }

    #[test]
    fn embedding_cost_uses_embedding_price_when_set() {
        let mut p = pricing();
        p.embedding_price_per_million = Some(0.02);
        let usage = Usage::new(5_000_000, 0, 5_000_000);
        let cost = p.compute_cost_usd(usage, LlmEndpointKind::Embedding);
        assert!((cost - 0.10).abs() < 1e-9);
    }

    // ---------- Gap-fill tests (Plan Task 1.12: cost.rs) ----------

    /// Zero-token usage must produce a zero-cost result for both endpoint
    /// kinds, regardless of whether an embedding price is configured.
    #[test]
    fn compute_cost_is_zero_for_zero_token_usage() {
        let mut p = pricing();
        let zero = Usage::new(0, 0, 0);

        let chat = p.compute_cost_usd(zero, LlmEndpointKind::ChatCompletion);
        assert_eq!(chat, 0.0);

        let chat_tools = p.compute_cost_usd(zero, LlmEndpointKind::ChatCompletionWithTools);
        assert_eq!(chat_tools, 0.0);

        // Without an explicit embedding price (falls back to input price).
        let embed_fallback = p.compute_cost_usd(zero, LlmEndpointKind::Embedding);
        assert_eq!(embed_fallback, 0.0);

        // With an explicit embedding price.
        p.embedding_price_per_million = Some(0.02);
        let embed_explicit = p.compute_cost_usd(zero, LlmEndpointKind::Embedding);
        assert_eq!(embed_explicit, 0.0);
    }

    /// ChatCompletionWithTools must be priced identically to ChatCompletion.
    #[test]
    fn chat_with_tools_costs_same_as_chat() {
        let p = pricing();
        let usage = Usage::new(500_000, 250_000, 750_000);
        let chat = p.compute_cost_usd(usage, LlmEndpointKind::ChatCompletion);
        let tools = p.compute_cost_usd(usage, LlmEndpointKind::ChatCompletionWithTools);
        assert_eq!(chat, tools);
    }

    /// When the embedding price is missing, the fallback to `input_price_per_million`
    /// kicks in — there is no `Option`/`None` return in the current contract; the
    /// function returns the input-priced cost. This test pins that fallback
    /// behaviour (plan called this the "None or equivalent" branch).
    #[test]
    fn embedding_cost_falls_back_when_embedding_price_missing() {
        let mut p = pricing();
        p.input_price_per_million = 0.25;
        p.embedding_price_per_million = None;

        let usage = Usage::new(1_000_000, 0, 1_000_000);
        let cost = p.compute_cost_usd(usage, LlmEndpointKind::Embedding);
        // 1M * 0.25 / 1M = 0.25
        assert!((cost - 0.25).abs() < 1e-9, "expected 0.25, got {}", cost);
    }

    /// Precision edge case: very small prices on small token counts. The
    /// current `f64` contract should not collapse to zero for sub-cent costs.
    #[test]
    fn compute_cost_handles_very_small_values() {
        let mut p = pricing();
        p.input_price_per_million = 0.000_001;
        p.output_price_per_million = 0.000_002;

        // 100 prompt + 50 completion tokens at these prices
        // = (100 * 1e-6 + 50 * 2e-6) / 1e6
        // = (1e-4 + 1e-4) / 1e6  = 2e-10
        let usage = Usage::new(100, 50, 150);
        let cost = p.compute_cost_usd(usage, LlmEndpointKind::ChatCompletion);
        assert!(cost > 0.0, "very small cost must not collapse to 0");
        assert!(
            (cost - 2.0e-10).abs() < 1e-18,
            "expected ~2e-10, got {}",
            cost
        );
    }

    /// Precision edge case: very large token counts with realistic prices.
    /// Must stay finite and match the algebraic expectation within f64
    /// tolerance.
    #[test]
    fn compute_cost_handles_very_large_values() {
        let p = pricing(); // 0.15 / 0.60 per million
        // u32::MAX prompt + completion tokens
        let big = u32::MAX;
        let usage = Usage::new(big, big, big as u32);
        let cost = p.compute_cost_usd(usage, LlmEndpointKind::ChatCompletion);
        let expected = (big as f64) * 0.15 / 1_000_000.0
            + (big as f64) * 0.60 / 1_000_000.0;
        assert!(cost.is_finite(), "cost overflowed to non-finite: {cost}");
        // Relative tolerance: large magnitudes can drift a bit in the last bits.
        assert!(
            ((cost - expected) / expected).abs() < 1e-12,
            "expected ~{}, got {}",
            expected,
            cost
        );
    }
}
