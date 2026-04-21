/// Embedding service wrapper for vector generation.
///
/// Like `BrainAtlasLlmService`, this owns a `CostAccountant` and records a
/// `llm_call_usage` row for every successful embedding call.
use crate::cost_accounting::CostAccountant;
use crate::{Infra, ServiceError};
use domain::UsageContext;
use std::sync::Arc;
use std::time::Instant;

pub struct BrainAtlasEmbeddingService<I> {
    infra: Arc<I>,
    accountant: CostAccountant<I>,
}

impl<I> BrainAtlasEmbeddingService<I> {
    pub fn new(infra: Arc<I>) -> Self {
        let accountant = CostAccountant::new(infra.clone());
        Self { infra, accountant }
    }
}

impl<E, I> BrainAtlasEmbeddingService<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E> + 'static,
{
    /// Generate embedding for text.
    pub async fn generate_embedding(
        &self,
        text: &str,
        model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<Vec<f32>, ServiceError<E>> {
        let api_key = self
            .infra
            .get("OPENROUTER_API_KEY")
            .map_err(ServiceError::InfraError)?;
        let embedding_model = match model_override {
            Some(m) => m.to_string(),
            None => self
                .infra
                .get("EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".to_string()),
        };

        let started = Instant::now();
        let ctx = ctx.with_caller_tag("embed");
        let outcome = self
            .infra
            .generate_embedding(&api_key, &embedding_model, text)
            .await
            .map_err(ServiceError::InfraError);
        self.accountant.finish(outcome, ctx, started).await
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `BrainAtlasEmbeddingService::generate_embedding` covering
    //! env-resolution, model override, caller_tag injection, and error
    //! propagation — hand-rolled `MockInfra` that implements the full
    //! `Infra` umbrella (unused sub-traits panic via `unreachable!()`).
    use super::*;
    use crate::infra::{
        EmbeddingGenerator, EnvInfra, LlmClient, LlmPricingRepo, LlmUsageRepo, Postgres, Query,
        QueryResult, S3Storage, VectorDatabase,
    };
    use domain::{
        ChunkSource, ExistingSummary, LlmCallOutcome, LlmEndpointKind, LlmPricing, LlmResponse,
        NewEmbedding, NewLlmCallUsage, NewRegionSummary, SimilarChunk, Usage, UsageAggregate,
        UsageAggregateFilter,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Debug, thiserror::Error)]
    #[error("mock error: {0}")]
    struct MockErr(&'static str);

    type EmbeddingResult = Result<(Vec<f32>, String), &'static str>;

    struct MockInfra {
        env: HashMap<String, String>,
        pricing: Option<LlmPricing>,
        records: Mutex<Vec<NewLlmCallUsage>>,
        /// Canned `generate_embedding` result. Every call consumes from the
        /// queue; on exhaustion returns a default vector.
        embedding_queue: Mutex<Vec<EmbeddingResult>>,
        /// Captured `(api_key, embedding_model, text)` tuples.
        calls: Mutex<Vec<(String, String, String)>>,
    }

    impl MockInfra {
        fn new() -> Self {
            let mut env = HashMap::new();
            env.insert("OPENROUTER_API_KEY".to_string(), "sk-test".to_string());
            env.insert("DATABASE_URL".to_string(), "postgres://mock".to_string());
            env.insert(
                "EMBEDDING_MODEL".to_string(),
                "text-embedding-3-small".to_string(),
            );
            Self {
                env,
                pricing: Some(LlmPricing {
                    model: "text-embedding-3-small".to_string(),
                    input_price_per_million: 0.10,
                    output_price_per_million: 0.0,
                    embedding_price_per_million: Some(0.02),
                    currency: "USD".to_string(),
                    effective_from: chrono::Utc::now(),
                }),
                records: Mutex::new(Vec::new()),
                embedding_queue: Mutex::new(Vec::new()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn enqueue_ok(&self, vec: Vec<f32>, model: &str) {
            self.embedding_queue
                .lock()
                .unwrap()
                .push(Ok((vec, model.to_string())));
        }

        fn enqueue_err(&self, msg: &'static str) {
            self.embedding_queue.lock().unwrap().push(Err(msg));
        }

        fn take_calls(&self) -> Vec<(String, String, String)> {
            std::mem::take(&mut *self.calls.lock().unwrap())
        }

        fn take_records(&self) -> Vec<NewLlmCallUsage> {
            std::mem::take(&mut *self.records.lock().unwrap())
        }
    }

    impl EnvInfra for MockInfra {
        type Error = MockErr;
        fn get(&self, key: &str) -> Result<String, Self::Error> {
            self.env.get(key).cloned().ok_or(MockErr("env missing"))
        }
    }

    #[async_trait::async_trait]
    impl EmbeddingGenerator for MockInfra {
        type Error = MockErr;
        async fn generate_embedding(
            &self,
            api_key: &str,
            embedding_model: &str,
            text: &str,
        ) -> Result<LlmCallOutcome<Vec<f32>>, Self::Error> {
            self.calls.lock().unwrap().push((
                api_key.to_string(),
                embedding_model.to_string(),
                text.to_string(),
            ));
            let mut q = self.embedding_queue.lock().unwrap();
            match q.pop() {
                Some(Ok((v, model))) => Ok(LlmCallOutcome::new(
                    v,
                    Usage::new(5, 0, 5),
                    model,
                    LlmEndpointKind::Embedding,
                )),
                Some(Err(m)) => Err(MockErr(m)),
                None => Ok(LlmCallOutcome::new(
                    vec![0.0, 0.0, 0.0],
                    Usage::new(1, 0, 1),
                    embedding_model.to_string(),
                    LlmEndpointKind::Embedding,
                )),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmPricingRepo for MockInfra {
        type Error = MockErr;
        async fn latest_for_model(
            &self,
            _db: &str,
            _model: &str,
        ) -> Result<Option<LlmPricing>, Self::Error> {
            Ok(self.pricing.clone())
        }
    }

    #[async_trait::async_trait]
    impl LlmUsageRepo for MockInfra {
        type Error = MockErr;
        async fn record(&self, _db: &str, row: NewLlmCallUsage) -> Result<(), Self::Error> {
            self.records.lock().unwrap().push(row);
            Ok(())
        }
        async fn aggregate(
            &self,
            _db: &str,
            _filter: UsageAggregateFilter,
        ) -> Result<UsageAggregate, Self::Error> {
            Ok(UsageAggregate::default())
        }
    }

    // ---- Unused sub-traits: panic if invoked ----
    #[async_trait::async_trait]
    impl Postgres for MockInfra {
        type Error = MockErr;
        async fn execute_query(&self, _db: &str, _q: Query) -> Result<QueryResult, Self::Error> {
            unreachable!("Postgres not used in embedding_service tests")
        }
    }
    #[async_trait::async_trait]
    impl S3Storage for MockInfra {
        type Error = MockErr;
        async fn download(&self, _key: &str) -> Result<String, Self::Error> {
            unreachable!()
        }
    }
    #[async_trait::async_trait]
    impl LlmClient for MockInfra {
        type Error = MockErr;
        async fn summarize_with_tools(
            &self,
            _api_key: &str,
            _chat_model: &str,
            _messages: &[serde_json::Value],
            _tools: &[serde_json::Value],
        ) -> Result<LlmCallOutcome<LlmResponse>, Self::Error> {
            unreachable!()
        }
        async fn generate_queries(
            &self,
            _api_key: &str,
            _chat_model: &str,
            _region_name: &str,
            _count: u32,
        ) -> Result<LlmCallOutcome<Vec<String>>, Self::Error> {
            unreachable!()
        }
    }
    #[async_trait::async_trait]
    impl VectorDatabase for MockInfra {
        type Error = MockErr;
        async fn insert_embeddings(
            &self,
            _db: &str,
            _e: Vec<NewEmbedding>,
        ) -> Result<(), Self::Error> {
            unreachable!()
        }
        async fn insert_summary(
            &self,
            _db: &str,
            _s: NewRegionSummary,
        ) -> Result<Uuid, Self::Error> {
            unreachable!()
        }
        async fn check_content_hash(
            &self,
            _db: &str,
            _region_id: i32,
            _hash: &str,
        ) -> Result<Option<ExistingSummary>, Self::Error> {
            unreachable!()
        }
        async fn search_similar(
            &self,
            _db: &str,
            _emb: Vec<f32>,
            _region_id: i32,
            _top_k: usize,
        ) -> Result<Vec<SimilarChunk>, Self::Error> {
            unreachable!()
        }
        async fn update_summary_text(
            &self,
            _db: &str,
            _id: Uuid,
            _text: &str,
        ) -> Result<(), Self::Error> {
            unreachable!()
        }
        async fn get_chunk_source(
            &self,
            _db: &str,
            _chunk_id: Uuid,
        ) -> Result<Option<ChunkSource>, Self::Error> {
            unreachable!()
        }
    }

    // ---- Tests ----

    #[tokio::test]
    async fn generate_embedding_uses_env_default_when_no_override() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_ok(vec![1.0, 2.0, 3.0], "text-embedding-3-small");
        let svc = BrainAtlasEmbeddingService::new(infra.clone());

        let v = svc
            .generate_embedding("hello", None, UsageContext::default())
            .await
            .expect("embedding generated");
        assert_eq!(v, vec![1.0, 2.0, 3.0]);

        let calls = infra.take_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "sk-test", "api key forwarded");
        assert_eq!(calls[0].1, "text-embedding-3-small", "env model used");
        assert_eq!(calls[0].2, "hello", "text forwarded unchanged");

        // Caller tag defaults to "embed".
        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].caller_tag.as_deref(), Some("embed"));
        assert_eq!(rows[0].endpoint, "embedding");
    }

    #[tokio::test]
    async fn generate_embedding_uses_model_override() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_ok(vec![0.5], "override-model");
        let svc = BrainAtlasEmbeddingService::new(infra.clone());

        let _ = svc
            .generate_embedding("abc", Some("override-model"), UsageContext::default())
            .await
            .unwrap();

        let calls = infra.take_calls();
        assert_eq!(calls[0].1, "override-model");
    }

    #[tokio::test]
    async fn generate_embedding_falls_back_to_hardcoded_when_env_missing() {
        // Remove EMBEDDING_MODEL to exercise the unwrap_or_else fallback.
        let mut infra = MockInfra::new();
        infra.env.remove("EMBEDDING_MODEL");
        let infra = Arc::new(infra);
        infra.enqueue_ok(vec![0.0], "text-embedding-3-small");
        let svc = BrainAtlasEmbeddingService::new(infra.clone());

        let _ = svc
            .generate_embedding("txt", None, UsageContext::default())
            .await
            .unwrap();

        let calls = infra.take_calls();
        assert_eq!(
            calls[0].1, "text-embedding-3-small",
            "hardcoded default used"
        );
    }

    #[tokio::test]
    async fn generate_embedding_surfaces_api_key_missing_error() {
        let mut infra = MockInfra::new();
        infra.env.remove("OPENROUTER_API_KEY");
        let infra = Arc::new(infra);
        let svc = BrainAtlasEmbeddingService::new(infra.clone());

        let err = svc
            .generate_embedding("txt", None, UsageContext::default())
            .await
            .expect_err("must fail when API key absent");
        match err {
            ServiceError::InfraError(_) => {}
            other => panic!("expected InfraError, got {other:?}"),
        }
        // No row recorded.
        assert!(infra.take_records().is_empty());
    }

    #[tokio::test]
    async fn generate_embedding_propagates_downstream_error_no_usage_row() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_err("rate-limited");
        let svc = BrainAtlasEmbeddingService::new(infra.clone());

        let err = svc
            .generate_embedding("txt", None, UsageContext::default())
            .await
            .expect_err("must propagate LLM error");
        match err {
            ServiceError::InfraError(e) => assert!(e.to_string().contains("rate-limited")),
            other => panic!("expected InfraError, got {other:?}"),
        }
        // Error path -> cost accountant is NOT invoked.
        assert!(infra.take_records().is_empty());
    }

    #[tokio::test]
    async fn generate_embedding_preserves_upstream_usage_context() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_ok(vec![1.0, 2.0], "text-embedding-3-small");
        let svc = BrainAtlasEmbeddingService::new(infra.clone());

        let region_id: i32 = 99;
        let ctx = UsageContext::default()
            .with_correlation(Some("corr-embed".to_string()))
            .with_region(Some(region_id));

        svc.generate_embedding("txt", None, ctx).await.unwrap();
        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].correlation_id.as_deref(),
            Some("corr-embed"),
            "correlation_id threaded through",
        );
        assert_eq!(rows[0].region_id, Some(region_id));
        // caller_tag is overwritten by the service layer to "embed".
        assert_eq!(rows[0].caller_tag.as_deref(), Some("embed"));
    }
}
