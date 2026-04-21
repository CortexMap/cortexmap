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
            ConfigKey::EvalVersion => "v0.3.0",
            ConfigKey::EvalConcurrency => "5",
            ConfigKey::EvalJudgeChatModel => "openai/gpt-4o-mini",
            ConfigKey::EvalRubricChatModel => "openai/gpt-4o",
            ConfigKey::EvalEmbeddingModel => "text-embedding-3-small",
            ConfigKey::EvalTopKChunks => "8",
            // 0.0 = no absolute floor; trust pgvector's ORDER BY similarity LIMIT top_k
            // to return the k best chunks, and let the judge LLM decide support.
            // The judge has explicit "partial"/"unsupported" verdicts for weak evidence,
            // so an SQL-level cutoff above ~0.35 just silently discards legitimate matches.
            ConfigKey::EvalSimilarityThreshold => "0.0",
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

    /// Every `ConfigKey` stringifies to exact snake_case with no stray
    /// capitals, hyphens or whitespace — operators grep for these names in
    /// the DB-backed config table, so a typo here is a silent regression.
    #[test]
    fn config_key_strings_are_snake_case() {
        assert_eq!(<&'static str>::from(ConfigKey::EvalVersion), "eval_version");
        assert_eq!(
            <&'static str>::from(ConfigKey::EvalConcurrency),
            "eval_concurrency"
        );
        assert_eq!(
            <&'static str>::from(ConfigKey::EvalJudgeChatModel),
            "eval_judge_chat_model"
        );
        assert_eq!(
            <&'static str>::from(ConfigKey::EvalRubricChatModel),
            "eval_rubric_chat_model"
        );
        assert_eq!(
            <&'static str>::from(ConfigKey::EvalEmbeddingModel),
            "eval_embedding_model"
        );
        assert_eq!(
            <&'static str>::from(ConfigKey::EvalTopKChunks),
            "eval_top_k_chunks"
        );
        assert_eq!(
            <&'static str>::from(ConfigKey::EvalSimilarityThreshold),
            "eval_similarity_threshold"
        );
        assert_eq!(
            <&'static str>::from(ConfigKey::BrainatlasBaseUrl),
            "brainatlas_base_url"
        );
    }

    /// Numeric defaults must parse into the types the services layer
    /// expects. If someone edits `default_value()` to a non-numeric string
    /// for a numeric key, this test turns that into a compile-time
    /// test-failure rather than a runtime panic at service startup.
    #[test]
    fn numeric_defaults_parse_to_expected_types() {
        // EvalConcurrency → u32
        let c: u32 = ConfigKey::EvalConcurrency
            .default_value()
            .parse()
            .expect("eval_concurrency default must parse as u32");
        assert!(c > 0, "concurrency default must be > 0");

        // EvalTopKChunks → i64 (matches retrieve_chunks_for_summary sig)
        let k: i64 = ConfigKey::EvalTopKChunks
            .default_value()
            .parse()
            .expect("eval_top_k_chunks default must parse as i64");
        assert!(k > 0, "top_k default must be > 0");

        // EvalSimilarityThreshold → f32 in [0.0, 1.0]
        let t: f32 = ConfigKey::EvalSimilarityThreshold
            .default_value()
            .parse()
            .expect("eval_similarity_threshold default must parse as f32");
        assert!(
            (0.0..=1.0).contains(&t),
            "similarity threshold default out of [0,1]: {}",
            t
        );
    }

    /// The eval-version default must match the `v<major>.<minor>.<patch>`
    /// convention used as the cache-key version; downstream migrations key
    /// on this literal, so we lock the shape (not the exact version).
    #[test]
    fn eval_version_default_is_semver_ish() {
        let v = ConfigKey::EvalVersion.default_value();
        assert!(
            v.starts_with('v'),
            "eval_version default should start with 'v': {}",
            v
        );
        let rest = &v[1..];
        assert_eq!(
            rest.split('.').count(),
            3,
            "expected v<major>.<minor>.<patch>, got {}",
            v
        );
        for part in rest.split('.') {
            part.parse::<u32>()
                .unwrap_or_else(|_| panic!("version component {:?} not numeric in {}", part, v));
        }
    }

    /// Brainatlas URL default must parse as an HTTP(S) URL-like string —
    /// the service layer will `reqwest::get` this value, so a bare host
    /// like "localhost:8082" would silently fail at runtime.
    #[test]
    fn brainatlas_base_url_default_has_scheme() {
        let url = ConfigKey::BrainatlasBaseUrl.default_value();
        assert!(
            url.starts_with("http://") || url.starts_with("https://"),
            "brainatlas_base_url default missing scheme: {}",
            url
        );
    }

    /// Unknown strings must fail parsing rather than silently defaulting
    /// to some variant. Operators rely on parse errors to spot typos in
    /// the config table.
    #[test]
    fn parsing_unknown_key_errors() {
        assert!("not_a_real_key".parse::<ConfigKey>().is_err());
        assert!("".parse::<ConfigKey>().is_err());
        // Wrong case: strum's snake_case matcher is case-sensitive.
        assert!("EvalVersion".parse::<ConfigKey>().is_err());
        assert!("EVAL_VERSION".parse::<ConfigKey>().is_err());
    }

    /// Display must be a clean snake_case without surrounding whitespace —
    /// matches `IntoStaticStr`. (Guards against someone accidentally
    /// customising `Display` and diverging from the DB column values.)
    #[test]
    fn display_matches_static_str() {
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
            let displayed = format!("{}", key);
            let static_s: &'static str = key.into();
            assert_eq!(displayed, static_s, "Display diverged from IntoStaticStr");
        }
    }
}
