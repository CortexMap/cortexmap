//! Tunable knobs surfaced via the eval config API.
//!
//! Mirrors the orch `ConfigKey` enum pattern: every value is sourced from a
//! database-backed config table at runtime so operators can adjust without
//! restarts.

use strum::{Display, EnumString, IntoStaticStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum ConfigKey {
    /// Cache-key version. Bump to force re-evaluation of every summary.
    EvalVersion,
    /// Max parallel scorings.
    EvalConcurrency,
    /// Cheap chat model (used for claim extraction + groundedness judging).
    EvalJudgeChatModel,
    /// Stronger chat model (used for the rubric judge).
    EvalRubricChatModel,
    /// Embedding model for claim retrieval. MUST match the model that
    /// generated `brain_region_embeddings` for similarity to be meaningful.
    EvalEmbeddingModel,
    /// Top-K chunks retrieved per claim for the groundedness judge.
    EvalTopKChunks,
    /// Reject retrieved chunks below this cosine similarity score.
    EvalSimilarityThreshold,
    /// Base URL for the brainatlas-be service.
    BrainatlasBaseUrl,
}

impl ConfigKey {
    pub fn default_value(self) -> &'static str {
        match self {
            ConfigKey::EvalVersion => "v1.0",
            ConfigKey::EvalConcurrency => "5",
            ConfigKey::EvalJudgeChatModel => "openai/gpt-4o-mini",
            ConfigKey::EvalRubricChatModel => "openai/gpt-4o",
            ConfigKey::EvalEmbeddingModel => "text-embedding-3-small",
            ConfigKey::EvalTopKChunks => "5",
            ConfigKey::EvalSimilarityThreshold => "0.6",
            ConfigKey::BrainatlasBaseUrl => "http://localhost:8082",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_keys_round_trip() {
        for key in [
            ConfigKey::EvalVersion,
            ConfigKey::EvalConcurrency,
            ConfigKey::EvalJudgeChatModel,
            ConfigKey::EvalRubricChatModel,
            ConfigKey::EvalEmbeddingModel,
            ConfigKey::EvalTopKChunks,
            ConfigKey::EvalSimilarityThreshold,
            ConfigKey::BrainatlasBaseUrl,
        ] {
            let s: &'static str = key.into();
            let back: ConfigKey = s.parse().unwrap();
            assert_eq!(back, key);
            assert!(!key.default_value().is_empty());
        }
    }
}
