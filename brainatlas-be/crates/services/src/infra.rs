use domain::{
    BrainRegionEntry, ChunkSource, ExistingSummary, LlmCallOutcome, LlmPricing, LlmProvider,
    LlmResponse, NewEmbedding, NewLlmCallUsage, NewRegionSummary, RegionMapping,
    RetrievalScope, SimilarChunk, UsageAggregate, UsageAggregateFilter,
};
use uuid::Uuid;

use crate::ServiceError;

/// Default OpenRouter base URL (no trailing slash).
pub const OPENROUTER_DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
/// Default Requesty base URL (no trailing slash).
pub const REQUESTY_DEFAULT_BASE_URL: &str = "https://router.requesty.ai/v1";

/// Outcome of `resolve_llm_provider`: which gateway to call, the credential
/// to send, and the base URL to prefix onto `/chat/completions` /
/// `/embeddings`.
#[derive(Debug, Clone)]
pub struct ResolvedLlmProvider {
    pub provider: LlmProvider,
    pub api_key: String,
    pub base_url: String,
}

/// Resolve which OpenAI-compatible LLM gateway to use for the next call.
///
/// Precedence (Requesty wins when both are set):
///   1. `REQUESTY_API_KEY` set    → Requesty, base URL from `REQUESTY_BASE_URL`
///      or [`REQUESTY_DEFAULT_BASE_URL`].
///   2. `OPENROUTER_API_KEY` set → OpenRouter, base URL from
///      `OPENROUTER_BASE_URL` or [`OPENROUTER_DEFAULT_BASE_URL`].
///   3. Neither set               → `ServiceError::Other(...)` with a message
///      naming both env vars.
///
/// An empty-string value is treated as unset so `docker compose` can safely
/// pass `REQUESTY_API_KEY: ${REQUESTY_API_KEY:-}` without accidentally flipping
/// the active provider.
pub fn resolve_llm_provider<I: EnvInfra>(
    infra: &I,
) -> Result<ResolvedLlmProvider, ServiceError<I::Error>> {
    fn non_empty(r: Result<String, impl std::error::Error>) -> Option<String> {
        r.ok().filter(|s| !s.is_empty())
    }

    if let Some(api_key) = non_empty(infra.get("REQUESTY_API_KEY")) {
        let base_url = non_empty(infra.get("REQUESTY_BASE_URL"))
            .unwrap_or_else(|| REQUESTY_DEFAULT_BASE_URL.to_string());
        return Ok(ResolvedLlmProvider {
            provider: LlmProvider::Requesty,
            api_key,
            base_url,
        });
    }
    if let Some(api_key) = non_empty(infra.get("OPENROUTER_API_KEY")) {
        let base_url = non_empty(infra.get("OPENROUTER_BASE_URL"))
            .unwrap_or_else(|| OPENROUTER_DEFAULT_BASE_URL.to_string());
        return Ok(ResolvedLlmProvider {
            provider: LlmProvider::OpenRouter,
            api_key,
            base_url,
        });
    }
    Err(ServiceError::Other(
        "missing LLM provider credentials: set REQUESTY_API_KEY or OPENROUTER_API_KEY".to_string(),
    ))
}

/// All queries the service layer can issue against Postgres.
pub enum Query {
    /// Fetch all rows from `region_mapping`, ordered by `structure_order`.
    ListRegions,
    /// Fetch a single region by UUID primary key.
    GetRegionById(Uuid),
    /// Check whether a row with the given UUID exists.
    RegionExists(Uuid),
}

/// Typed results returned by each query variant.
pub enum QueryResult {
    Regions(Vec<RegionMapping>),
    Region(Vec<BrainRegionEntry>),
    Exists(bool),
}

/// Postgres infra trait — accepts a typed query and executes it.
/// Connection management and DB row conversion are entirely internal to the implementation.
#[async_trait::async_trait]
pub trait Postgres: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn execute_query(
        &self,
        database_uri: &str,
        query: Query,
    ) -> Result<QueryResult, Self::Error>;
}

pub trait EnvInfra {
    type Error: std::error::Error + Send + Sync + 'static;
    fn get(&self, key: &str) -> Result<String, Self::Error>;
}

/// Blanket: any `T: Postgres` automatically satisfies `Infra`.
pub trait Infra:
    Postgres<Error = <Self as Infra>::Error>
    + EnvInfra<Error = <Self as Infra>::Error>
    + S3Storage<Error = <Self as Infra>::Error>
    + EmbeddingGenerator<Error = <Self as Infra>::Error>
    + LlmClient<Error = <Self as Infra>::Error>
    + VectorDatabase<Error = <Self as Infra>::Error>
    + LlmPricingRepo<Error = <Self as Infra>::Error>
    + LlmUsageRepo<Error = <Self as Infra>::Error>
{
    type Error: std::error::Error + Send + Sync + 'static;
}

impl<E, T> Infra for T
where
    T: Postgres<Error = E>
        + EnvInfra<Error = E>
        + S3Storage<Error = E>
        + EmbeddingGenerator<Error = E>
        + LlmClient<Error = E>
        + VectorDatabase<Error = E>
        + LlmPricingRepo<Error = E>
        + LlmUsageRepo<Error = E>,
    E: std::error::Error + Send + Sync + 'static,
{
    type Error = E;
}

/// S3 credentials for self-hosted S3-compatible storage
#[derive(Debug, Clone)]
pub struct S3Creds {
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
}

/// S3 storage access
#[async_trait::async_trait]
pub trait S3Storage: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Download file from S3 as UTF-8 string (reads credentials from env internally)
    async fn download(&self, key: &str) -> Result<String, Self::Error>;
}

/// Embedding generation
#[async_trait::async_trait]
pub trait EmbeddingGenerator: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Generate embedding for text chunk. Returns the embedding together with
    /// provider-reported token usage so the accounting helper can persist a
    /// cost row. Implementations that cannot obtain usage must still return
    /// `LlmCallOutcome` with `Usage::default()` and log a warning.
    ///
    /// `base_url` is the gateway's API base (e.g. `https://openrouter.ai/api/v1`
    /// or `https://router.requesty.ai/v1`) — no trailing slash. See
    /// [`resolve_llm_provider`] for how callers derive it.
    async fn generate_embedding(
        &self,
        base_url: &str,
        api_key: &str,
        embedding_model: &str,
        text: &str,
    ) -> Result<LlmCallOutcome<Vec<f32>>, Self::Error>;
}

/// LLM client for text generation
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Send a chat completion request with tool definitions, returning either
    /// tool calls the LLM wants to make or the final text response — together
    /// with provider-reported token usage.
    ///
    /// `base_url` is the gateway's API base — see
    /// [`EmbeddingGenerator::generate_embedding`].
    async fn summarize_with_tools(
        &self,
        base_url: &str,
        api_key: &str,
        chat_model: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<LlmCallOutcome<LlmResponse>, Self::Error>;

    /// Generate search queries for a brain region. The returned `Usage` is
    /// aggregated across the internal iterations of the tool-calling loop
    /// (see `OpenAiCompatibleClient::generate_queries`).
    ///
    /// `base_url` is the gateway's API base — see
    /// [`EmbeddingGenerator::generate_embedding`].
    async fn generate_queries(
        &self,
        base_url: &str,
        api_key: &str,
        chat_model: &str,
        region_name: &str,
        count: u32,
    ) -> Result<LlmCallOutcome<Vec<String>>, Self::Error>;
}

/// Vector database operations
#[async_trait::async_trait]
pub trait VectorDatabase: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Insert embeddings in bulk
    async fn insert_embeddings(
        &self,
        database_url: &str,
        embeddings: Vec<NewEmbedding>,
    ) -> Result<(), Self::Error>;

    /// Insert region summary and return ID
    async fn insert_summary(
        &self,
        database_url: &str,
        summary: NewRegionSummary,
    ) -> Result<Uuid, Self::Error>;

    /// Check if content hash already exists
    async fn check_content_hash(
        &self,
        database_url: &str,
        region_id: i32,
        content_hash: &str,
    ) -> Result<Option<ExistingSummary>, Self::Error>;

    /// Search for similar chunks by embedding vector, scoped to a region and
    /// summary, with an explicit fallback policy.
    async fn search_similar(
        &self,
        database_url: &str,
        query_embedding: Vec<f32>,
        retrieval_scope: RetrievalScope,
        top_k: usize,
    ) -> Result<Vec<SimilarChunk>, Self::Error>;

    /// Update the summary text for an existing summary record
    async fn update_summary_text(
        &self,
        database_url: &str,
        summary_id: Uuid,
        summary_text: &str,
    ) -> Result<(), Self::Error>;

    /// Get full source details for a chunk by its UUID
    async fn get_chunk_source(
        &self,
        database_url: &str,
        chunk_id: Uuid,
    ) -> Result<Option<ChunkSource>, Self::Error>;
}

/// Read-only access to the `llm_pricing` catalogue. One row per (model,
/// effective_from); the repo returns the latest one for a given model.
#[async_trait::async_trait]
pub trait LlmPricingRepo: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn latest_for_model(
        &self,
        database_url: &str,
        model: &str,
    ) -> Result<Option<LlmPricing>, Self::Error>;
}

/// Persistence port for `llm_call_usage`. One row is recorded per logical LLM
/// call (multi-iteration calls are aggregated before persistence).
#[async_trait::async_trait]
pub trait LlmUsageRepo: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn record(&self, database_url: &str, row: NewLlmCallUsage) -> Result<(), Self::Error>;

    async fn aggregate(
        &self,
        database_url: &str,
        filter: UsageAggregateFilter,
    ) -> Result<UsageAggregate, Self::Error>;
}

#[cfg(test)]
mod resolver_tests {
    //! Tests for [`resolve_llm_provider`] — covers precedence, defaulting,
    //! explicit base-URL override, and the "no keys set" error surface.
    use super::*;
    use std::collections::HashMap;

    #[derive(Debug, thiserror::Error)]
    #[error("mock env: missing {0}")]
    struct MockEnvErr(String);

    struct MockEnv {
        vars: HashMap<String, String>,
    }

    impl EnvInfra for MockEnv {
        type Error = MockEnvErr;
        fn get(&self, key: &str) -> Result<String, Self::Error> {
            self.vars
                .get(key)
                .cloned()
                .ok_or_else(|| MockEnvErr(key.to_string()))
        }
    }

    fn env(pairs: &[(&str, &str)]) -> MockEnv {
        MockEnv {
            vars: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn requesty_wins_when_both_keys_set() {
        let infra = env(&[
            ("REQUESTY_API_KEY", "req-key"),
            ("OPENROUTER_API_KEY", "or-key"),
        ]);
        let r = resolve_llm_provider(&infra).expect("resolves");
        assert_eq!(r.provider, LlmProvider::Requesty);
        assert_eq!(r.api_key, "req-key");
        assert_eq!(r.base_url, REQUESTY_DEFAULT_BASE_URL);
    }

    #[test]
    fn openrouter_used_when_only_openrouter_set() {
        let infra = env(&[("OPENROUTER_API_KEY", "or-key")]);
        let r = resolve_llm_provider(&infra).expect("resolves");
        assert_eq!(r.provider, LlmProvider::OpenRouter);
        assert_eq!(r.api_key, "or-key");
        assert_eq!(r.base_url, OPENROUTER_DEFAULT_BASE_URL);
    }

    #[test]
    fn requesty_used_when_only_requesty_set() {
        let infra = env(&[("REQUESTY_API_KEY", "req-key")]);
        let r = resolve_llm_provider(&infra).expect("resolves");
        assert_eq!(r.provider, LlmProvider::Requesty);
        assert_eq!(r.api_key, "req-key");
        assert_eq!(r.base_url, REQUESTY_DEFAULT_BASE_URL);
    }

    #[test]
    fn explicit_requesty_base_url_override_honored() {
        let infra = env(&[
            ("REQUESTY_API_KEY", "req-key"),
            ("REQUESTY_BASE_URL", "https://custom.requesty.example/v1"),
        ]);
        let r = resolve_llm_provider(&infra).expect("resolves");
        assert_eq!(r.provider, LlmProvider::Requesty);
        assert_eq!(r.base_url, "https://custom.requesty.example/v1");
    }

    #[test]
    fn explicit_openrouter_base_url_override_honored() {
        let infra = env(&[
            ("OPENROUTER_API_KEY", "or-key"),
            ("OPENROUTER_BASE_URL", "https://or-proxy.example/v1"),
        ]);
        let r = resolve_llm_provider(&infra).expect("resolves");
        assert_eq!(r.provider, LlmProvider::OpenRouter);
        assert_eq!(r.base_url, "https://or-proxy.example/v1");
    }

    #[test]
    fn empty_string_api_key_is_treated_as_unset() {
        // Matches the docker-compose pattern: ${REQUESTY_API_KEY:-} where an
        // unset host var expands to empty string. We must fall through to
        // OpenRouter in that case.
        let infra = env(&[("REQUESTY_API_KEY", ""), ("OPENROUTER_API_KEY", "or-key")]);
        let r = resolve_llm_provider(&infra).expect("resolves");
        assert_eq!(r.provider, LlmProvider::OpenRouter);
        assert_eq!(r.api_key, "or-key");
    }

    #[test]
    fn empty_base_url_falls_back_to_default() {
        // Same rationale as above: empty-string overrides from compose must
        // not override the default.
        let infra = env(&[
            ("OPENROUTER_API_KEY", "or-key"),
            ("OPENROUTER_BASE_URL", ""),
        ]);
        let r = resolve_llm_provider(&infra).expect("resolves");
        assert_eq!(r.base_url, OPENROUTER_DEFAULT_BASE_URL);
    }

    #[test]
    fn errors_when_neither_key_set() {
        let infra = env(&[]);
        let err = resolve_llm_provider(&infra).expect_err("must fail");
        match err {
            ServiceError::Other(msg) => {
                assert!(msg.contains("REQUESTY_API_KEY"), "{msg}");
                assert!(msg.contains("OPENROUTER_API_KEY"), "{msg}");
            }
            other => panic!("expected ServiceError::Other, got {other:?}"),
        }
    }
}
