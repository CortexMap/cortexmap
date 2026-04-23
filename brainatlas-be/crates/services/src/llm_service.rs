/// LLM service wrapper for text generation tasks.
///
/// Every public method:
/// 1. Resolves API key and chat model from env.
/// 2. Calls the infra `LlmClient`, receiving an `LlmCallOutcome`.
/// 3. Hands the outcome to the shared `CostAccountant`, which persists a row
///    in `llm_call_usage` and emits the `llm.call` tracing event.
/// 4. Returns the unwrapped business value.
///
/// `UsageContext` is provided by the caller (the app layer) and carries the
/// `caller_tag`, correlation id and any region/summary/batch linkage.
use crate::cost_accounting::CostAccountant;
use crate::infra::resolve_llm_provider;
use crate::{Infra, ServiceError};
use domain::{ClaimsResponse, GroundednessVerdict, LlmResponse, RubricScores, UsageContext};
use std::sync::Arc;
use std::time::Instant;
use tracing::warn;

pub struct BrainAtlasLlmService<I> {
    infra: Arc<I>,
    accountant: CostAccountant<I>,
}

impl<I> BrainAtlasLlmService<I> {
    pub fn new(infra: Arc<I>) -> Self {
        let accountant = CostAccountant::new(infra.clone());
        Self { infra, accountant }
    }
}

impl<E, I> BrainAtlasLlmService<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E> + 'static,
{
    /// Send a multi-turn chat with tool definitions, returning tool calls or final text.
    pub async fn summarize_with_tools(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<LlmResponse, ServiceError<E>> {
        let resolved = resolve_llm_provider(&*self.infra)?;
        let chat_model = match chat_model_override {
            Some(m) => m.to_string(),
            None => self
                .infra
                .get("CHAT_MODEL")
                .unwrap_or_else(|_| "openai/gpt-4o-mini".to_string()),
        };

        let started = Instant::now();
        let ctx = if ctx.caller_tag.is_some() {
            ctx
        } else {
            ctx.with_caller_tag(if tools.is_empty() {
                "chat"
            } else {
                "rag_summarize"
            })
        }
        .with_provider(resolved.provider);
        let outcome = self
            .infra
            .summarize_with_tools(
                &resolved.base_url,
                &resolved.api_key,
                &chat_model,
                messages,
                tools,
            )
            .await
            .map_err(ServiceError::InfraError);
        self.accountant.finish(outcome, ctx, started).await
    }

    /// Generate search queries for a brain region.
    pub async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
        ctx: UsageContext,
    ) -> Result<Vec<String>, ServiceError<E>> {
        let resolved = resolve_llm_provider(&*self.infra)?;
        let chat_model = self
            .infra
            .get("CHAT_MODEL")
            .unwrap_or_else(|_| "openai/gpt-4o-mini".to_string());

        let started = Instant::now();
        let ctx = if ctx.caller_tag.is_some() {
            ctx
        } else {
            ctx.with_caller_tag("generate_queries")
        }
        .with_provider(resolved.provider);
        let outcome = self
            .infra
            .generate_queries(
                &resolved.base_url,
                &resolved.api_key,
                &chat_model,
                region_name,
                count,
            )
            .await
            .map_err(ServiceError::InfraError);
        self.accountant.finish(outcome, ctx, started).await
    }

    /// Run a single-turn structured-output chat: system + user, no tools.
    /// Returns the model's final text. Does not handle tool calls (rejects them as an error).
    async fn structured_chat(
        &self,
        system_prompt: &str,
        user_content: &str,
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<String, ServiceError<E>> {
        let messages = vec![
            serde_json::json!({"role": "system", "content": system_prompt}),
            serde_json::json!({"role": "user",   "content": user_content}),
        ];
        match self
            .summarize_with_tools(&messages, &[], chat_model_override, ctx)
            .await?
        {
            LlmResponse::Final(text) => Ok(text),
            LlmResponse::ToolCalls(_) => {
                warn!("structured_chat received unexpected tool calls; treating as empty response");
                Err(ServiceError::Other(
                    "LLM returned tool calls instead of final text for structured prompt"
                        .to_string(),
                ))
            }
        }
    }

    /// Extract atomic claims from a summary. Single LLM call returning structured JSON.
    pub async fn extract_claims(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<ClaimsResponse, ServiceError<E>> {
        let system = EXTRACT_CLAIMS_SYSTEM.replace("{{REGION_NAME}}", region_name);
        let user = format!(
            "Brain region: {}\n\nSummary:\n\n{}",
            region_name, summary_text
        );
        let ctx = ctx.with_caller_tag("extract_claims");
        let raw = self
            .structured_chat(&system, &user, chat_model_override, ctx)
            .await?;
        parse_json_loose::<ClaimsResponse>(&raw)
            .map_err(|e| ServiceError::Other(format!("extract_claims parse error: {e}")))
    }

    /// Judge whether a single claim is grounded in the supplied evidence chunks.
    pub async fn judge_groundedness(
        &self,
        claim_text: &str,
        evidence_chunks: &[String],
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<GroundednessVerdict, ServiceError<E>> {
        let mut user = String::new();
        user.push_str("Claim:\n");
        user.push_str(claim_text);
        user.push_str("\n\nEvidence chunks:\n");
        for (idx, chunk) in evidence_chunks.iter().enumerate() {
            user.push_str(&format!("\n[{}] {}\n", idx + 1, chunk));
        }
        if evidence_chunks.is_empty() {
            user.push_str("\n(no evidence chunks)\n");
        }
        let ctx = ctx.with_caller_tag("judge_groundedness");
        let raw = self
            .structured_chat(JUDGE_GROUNDEDNESS_SYSTEM, &user, chat_model_override, ctx)
            .await?;
        parse_json_loose::<GroundednessVerdict>(&raw)
            .map_err(|e| ServiceError::Other(format!("judge_groundedness parse error: {e}")))
    }

    /// Score the summary against the fixed five-criterion rubric. Single LLM call.
    pub async fn judge_rubric(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<RubricScores, ServiceError<E>> {
        let system = JUDGE_RUBRIC_SYSTEM.replace("{{REGION_NAME}}", region_name);
        let user = format!(
            "Brain region: {}\n\nSummary:\n\n{}",
            region_name, summary_text
        );
        let ctx = ctx.with_caller_tag("judge_rubric");
        let raw = self
            .structured_chat(&system, &user, chat_model_override, ctx)
            .await?;
        parse_json_loose::<RubricScores>(&raw)
            .map_err(|e| ServiceError::Other(format!("judge_rubric parse error: {e}")))
    }

    /// Judge whether a single cited chunk actually supports the attached claim.
    ///
    /// Distinct from `judge_groundedness`: here we are not asking "is there
    /// evidence for this claim?" but "did the author cite the correct chunk?".
    /// Exactly ONE chunk is passed; the sentence from the original summary is
    /// included as context so the judge can see the surrounding rhetoric.
    pub async fn judge_citation(
        &self,
        claim_text: &str,
        sentence_context: &str,
        chunk_text: &str,
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<GroundednessVerdict, ServiceError<E>> {
        let user = format!(
            "Claim:\n{}\n\nSentence as written in summary:\n{}\n\nCited chunk:\n{}\n",
            claim_text, sentence_context, chunk_text
        );
        let ctx = ctx.with_caller_tag("judge_citation");
        let raw = self
            .structured_chat(JUDGE_CITATION_SYSTEM, &user, chat_model_override, ctx)
            .await?;
        parse_json_loose::<GroundednessVerdict>(&raw)
            .map_err(|e| ServiceError::Other(format!("judge_citation parse error: {e}")))
    }
}

// Prompt templates loaded at compile time.
const EXTRACT_CLAIMS_SYSTEM: &str = include_str!("../../app/prompts/extract_claims_system.md");
const JUDGE_GROUNDEDNESS_SYSTEM: &str =
    include_str!("../../app/prompts/judge_groundedness_system.md");
const JUDGE_RUBRIC_SYSTEM: &str = include_str!("../../app/prompts/judge_rubric_system.md");
const JUDGE_CITATION_SYSTEM: &str = include_str!("../../app/prompts/judge_citation_system.md");

/// Parse a JSON payload that may be wrapped in markdown fences or
/// surrounded by stray prose. Strips fences and falls back to the
/// outermost `{...}` substring before invoking serde.
fn parse_json_loose<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, serde_json::Error> {
    let trimmed = raw.trim();

    // Strip ```json ... ``` or ``` ... ``` fences if present.
    let stripped = if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.trim_start().trim_end_matches("```").trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.trim_start().trim_end_matches("```").trim()
    } else {
        trimmed
    };

    if let Ok(v) = serde_json::from_str::<T>(stripped) {
        return Ok(v);
    }

    // Fallback: extract the outermost balanced { ... } region.
    if let (Some(start), Some(end)) = (stripped.find('{'), stripped.rfind('}'))
        && start < end
    {
        return serde_json::from_str::<T>(&stripped[start..=end]);
    }

    // Force the original parse error to propagate.
    serde_json::from_str::<T>(stripped)
}

#[cfg(test)]
mod tests {
    //! Tests for `BrainAtlasLlmService` caller_tag semantics, parser failure
    //! modes, and LLM-error propagation. The `MockInfra` here implements the
    //! full `Infra` umbrella trait — methods that the service layer does NOT
    //! touch during these tests panic via `unreachable!()`.
    //!
    //! NOTE on semantics: `summarize_with_tools` (`llm_service.rs:57-65`) and
    //! `generate_queries` (`:91-95`) preserve a caller-provided `caller_tag`.
    //! `extract_claims`, `judge_groundedness`, `judge_rubric` and
    //! `judge_citation` unconditionally overwrite `caller_tag` via
    //! `UsageContext::with_caller_tag(...)` — so for those methods only the
    //! default-tag assertion is meaningful.
    use super::*;
    use crate::infra::{
        EmbeddingGenerator, EnvInfra, LlmClient, LlmPricingRepo, LlmUsageRepo, Postgres, Query,
        QueryResult, S3Storage, VectorDatabase,
    };
    use domain::{
        ChunkSource, ExistingSummary, LlmCallOutcome, LlmEndpointKind, LlmPricing, LlmResponse,
        NewEmbedding, NewLlmCallUsage, NewRegionSummary, RetrievalScope, SimilarChunk, Usage,
        UsageAggregate, UsageAggregateFilter, UsageContext,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Debug, thiserror::Error)]
    #[error("mock error: {0}")]
    struct MockErr(&'static str);

    /// Canned response for the next `LlmClient::summarize_with_tools` call.
    enum CannedSummarize {
        Ok(LlmCallOutcome<LlmResponse>),
        Err(&'static str),
    }

    /// Canned response for the next `LlmClient::generate_queries` call.
    enum CannedGenerate {
        Ok(LlmCallOutcome<Vec<String>>),
        Err(&'static str),
    }

    struct MockInfra {
        env: HashMap<String, String>,
        pricing: Option<LlmPricing>,
        /// Rows recorded by `LlmUsageRepo::record`.
        records: Mutex<Vec<NewLlmCallUsage>>,
        /// FIFO queue of canned `summarize_with_tools` responses.
        summarize_queue: Mutex<Vec<CannedSummarize>>,
        /// FIFO queue of canned `generate_queries` responses.
        generate_queue: Mutex<Vec<CannedGenerate>>,
        /// `(base_url, api_key)` captured for each `LlmClient` invocation,
        /// in call order. Lets tests assert Requesty vs OpenRouter routing.
        llm_calls: Mutex<Vec<(String, String)>>,
    }

    impl MockInfra {
        fn new() -> Self {
            let mut env = HashMap::new();
            env.insert("OPENROUTER_API_KEY".to_string(), "sk-test".to_string());
            env.insert("CHAT_MODEL".to_string(), "openai/gpt-4o-mini".to_string());
            env.insert("DATABASE_URL".to_string(), "postgres://mock".to_string());
            Self {
                env,
                pricing: Some(LlmPricing {
                    model: "openai/gpt-4o-mini".to_string(),
                    input_price_per_million: 0.15,
                    output_price_per_million: 0.60,
                    embedding_price_per_million: None,
                    currency: "USD".to_string(),
                    effective_from: chrono::Utc::now(),
                }),
                records: Mutex::new(Vec::new()),
                summarize_queue: Mutex::new(Vec::new()),
                generate_queue: Mutex::new(Vec::new()),
                llm_calls: Mutex::new(Vec::new()),
            }
        }

        fn enqueue_summarize_ok(&self, value: LlmResponse) {
            self.summarize_queue
                .lock()
                .unwrap()
                .push(CannedSummarize::Ok(LlmCallOutcome::new(
                    value,
                    Usage::new(10, 5, 15),
                    "openai/gpt-4o-mini".to_string(),
                    LlmEndpointKind::ChatCompletion,
                )));
        }

        fn enqueue_summarize_err(&self, msg: &'static str) {
            self.summarize_queue
                .lock()
                .unwrap()
                .push(CannedSummarize::Err(msg));
        }

        fn enqueue_generate_ok(&self, queries: Vec<String>) {
            self.generate_queue
                .lock()
                .unwrap()
                .push(CannedGenerate::Ok(LlmCallOutcome::new(
                    queries,
                    Usage::new(20, 10, 30),
                    "openai/gpt-4o-mini".to_string(),
                    LlmEndpointKind::ChatCompletionWithTools,
                )));
        }

        fn enqueue_generate_err(&self, msg: &'static str) {
            self.generate_queue
                .lock()
                .unwrap()
                .push(CannedGenerate::Err(msg));
        }

        fn take_records(&self) -> Vec<NewLlmCallUsage> {
            std::mem::take(&mut *self.records.lock().unwrap())
        }

        fn take_llm_calls(&self) -> Vec<(String, String)> {
            std::mem::take(&mut *self.llm_calls.lock().unwrap())
        }
    }

    impl EnvInfra for MockInfra {
        type Error = MockErr;
        fn get(&self, key: &str) -> Result<String, Self::Error> {
            self.env.get(key).cloned().ok_or(MockErr("env key missing"))
        }
    }

    #[async_trait::async_trait]
    impl LlmClient for MockInfra {
        type Error = MockErr;

        async fn summarize_with_tools(
            &self,
            base_url: &str,
            api_key: &str,
            _chat_model: &str,
            _messages: &[serde_json::Value],
            _tools: &[serde_json::Value],
        ) -> Result<LlmCallOutcome<LlmResponse>, Self::Error> {
            self.llm_calls
                .lock()
                .unwrap()
                .push((base_url.to_string(), api_key.to_string()));
            let mut q = self.summarize_queue.lock().unwrap();
            assert!(
                !q.is_empty(),
                "summarize_with_tools called with empty queue"
            );
            match q.remove(0) {
                CannedSummarize::Ok(o) => Ok(o),
                CannedSummarize::Err(m) => Err(MockErr(m)),
            }
        }

        async fn generate_queries(
            &self,
            base_url: &str,
            api_key: &str,
            _chat_model: &str,
            _region_name: &str,
            _count: u32,
        ) -> Result<LlmCallOutcome<Vec<String>>, Self::Error> {
            self.llm_calls
                .lock()
                .unwrap()
                .push((base_url.to_string(), api_key.to_string()));
            let mut q = self.generate_queue.lock().unwrap();
            assert!(!q.is_empty(), "generate_queries called with empty queue");
            match q.remove(0) {
                CannedGenerate::Ok(o) => Ok(o),
                CannedGenerate::Err(m) => Err(MockErr(m)),
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

    // ----- Unused sub-traits: panic if invoked -----

    #[async_trait::async_trait]
    impl Postgres for MockInfra {
        type Error = MockErr;
        async fn execute_query(&self, _db: &str, _q: Query) -> Result<QueryResult, Self::Error> {
            unreachable!("Postgres::execute_query not used in llm_service tests")
        }
    }

    #[async_trait::async_trait]
    impl S3Storage for MockInfra {
        type Error = MockErr;
        async fn download(&self, _key: &str) -> Result<String, Self::Error> {
            unreachable!("S3Storage::download not used in llm_service tests")
        }
    }

    #[async_trait::async_trait]
    impl EmbeddingGenerator for MockInfra {
        type Error = MockErr;
        async fn generate_embedding(
            &self,
            _base_url: &str,
            _api_key: &str,
            _model: &str,
            _text: &str,
        ) -> Result<LlmCallOutcome<Vec<f32>>, Self::Error> {
            unreachable!("EmbeddingGenerator::generate_embedding not used in llm_service tests")
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
            _retrieval_scope: RetrievalScope,
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

    // ----- Helpers -----

    fn make_service(infra: Arc<MockInfra>) -> BrainAtlasLlmService<MockInfra> {
        BrainAtlasLlmService::new(infra)
    }

    fn well_formed_claims_json() -> String {
        r#"{"claims":[{"id":1,"section":"Overview","text":"hi","cited_chunks":[]}]}"#.to_string()
    }

    fn well_formed_groundedness_json() -> String {
        r#"{"verdict":"supported","confidence":0.9,"supporting_chunks":[1],"rationale":"ok"}"#
            .to_string()
    }

    fn well_formed_rubric_json() -> String {
        r#"{
            "relevance":       {"score":5,"rationale":"a"},
            "coherence":       {"score":4,"rationale":"b"},
            "specificity":     {"score":3,"rationale":"c"},
            "clinical_utility":{"score":4,"rationale":"d"},
            "terminology":     {"score":5,"rationale":"e"}
        }"#
        .to_string()
    }

    // =============================================================
    // caller_tag preservation for summarize_with_tools
    // =============================================================

    #[tokio::test]
    async fn summarize_with_tools_preserves_caller_provided_tag() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final("answer".into()));
        let svc = make_service(infra.clone());

        let ctx = UsageContext::default().with_caller_tag("my-custom-tag");
        let resp = svc
            .summarize_with_tools(&[], &[], None, ctx)
            .await
            .expect("call succeeds");
        match resp {
            LlmResponse::Final(t) => assert_eq!(t, "answer"),
            _ => panic!("expected Final"),
        }

        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].caller_tag.as_deref(), Some("my-custom-tag"));
    }

    #[tokio::test]
    async fn summarize_with_tools_default_tag_when_missing_no_tools() {
        // No tools and no caller-provided tag => default is "chat".
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final("answer".into()));
        let svc = make_service(infra.clone());

        let ctx = UsageContext::default();
        let _ = svc
            .summarize_with_tools(&[], &[], None, ctx)
            .await
            .expect("call succeeds");

        let rows = infra.take_records();
        assert_eq!(rows[0].caller_tag.as_deref(), Some("chat"));
    }

    #[tokio::test]
    async fn summarize_with_tools_default_tag_when_missing_with_tools() {
        // Tools present and no caller-provided tag => default is "rag_summarize".
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final("answer".into()));
        let svc = make_service(infra.clone());

        let tool = serde_json::json!({"type":"function","function":{"name":"noop"}});
        let ctx = UsageContext::default();
        let _ = svc
            .summarize_with_tools(&[], &[tool], None, ctx)
            .await
            .expect("call succeeds");

        let rows = infra.take_records();
        assert_eq!(rows[0].caller_tag.as_deref(), Some("rag_summarize"));
    }

    // =============================================================
    // caller_tag for generate_queries
    // =============================================================

    #[tokio::test]
    async fn generate_queries_preserves_caller_provided_tag() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_generate_ok(vec!["q1".into(), "q2".into()]);
        let svc = make_service(infra.clone());

        let ctx = UsageContext::default().with_caller_tag("batch-queries");
        let out = svc
            .generate_queries("hippocampus", 2, ctx)
            .await
            .expect("call succeeds");
        assert_eq!(out, vec!["q1".to_string(), "q2".to_string()]);

        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].caller_tag.as_deref(), Some("batch-queries"));
    }

    #[tokio::test]
    async fn generate_queries_default_tag_when_missing() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_generate_ok(vec!["q".into()]);
        let svc = make_service(infra.clone());

        let _ = svc
            .generate_queries("hippocampus", 1, UsageContext::default())
            .await
            .expect("call succeeds");

        let rows = infra.take_records();
        assert_eq!(rows[0].caller_tag.as_deref(), Some("generate_queries"));
    }

    // =============================================================
    // Judge/extract methods: caller_tag is ALWAYS overridden by the method
    // default. Production code at `:145`, `:171`, `:192`, `:218` calls
    // `ctx.with_caller_tag(...)` unconditionally, so any caller-provided
    // tag is discarded. These tests pin that documented behaviour.
    // =============================================================

    #[tokio::test]
    async fn extract_claims_uses_method_default_tag() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final(well_formed_claims_json()));
        let svc = make_service(infra.clone());

        // Deliberately provide a different caller_tag — it must be overridden.
        let ctx = UsageContext::default().with_caller_tag("should-be-ignored");
        let resp = svc
            .extract_claims("summary text", "Hippocampus", None, ctx)
            .await
            .expect("extract_claims succeeds");
        assert_eq!(resp.claims.len(), 1);

        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].caller_tag.as_deref(), Some("extract_claims"));
    }

    #[tokio::test]
    async fn judge_groundedness_uses_method_default_tag() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final(well_formed_groundedness_json()));
        let svc = make_service(infra.clone());

        let ctx = UsageContext::default().with_caller_tag("ignored");
        let _ = svc
            .judge_groundedness("claim text", &["evidence-1".into()], None, ctx)
            .await
            .expect("judge_groundedness succeeds");

        let rows = infra.take_records();
        assert_eq!(rows[0].caller_tag.as_deref(), Some("judge_groundedness"));
    }

    #[tokio::test]
    async fn judge_rubric_uses_method_default_tag() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final(well_formed_rubric_json()));
        let svc = make_service(infra.clone());

        let ctx = UsageContext::default();
        let _ = svc
            .judge_rubric("summary", "Hippocampus", None, ctx)
            .await
            .expect("judge_rubric succeeds");

        let rows = infra.take_records();
        assert_eq!(rows[0].caller_tag.as_deref(), Some("judge_rubric"));
    }

    #[tokio::test]
    async fn judge_citation_uses_method_default_tag() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final(well_formed_groundedness_json()));
        let svc = make_service(infra.clone());

        let ctx = UsageContext::default();
        let _ = svc
            .judge_citation("claim", "sentence", "chunk text", None, ctx)
            .await
            .expect("judge_citation succeeds");

        let rows = infra.take_records();
        assert_eq!(rows[0].caller_tag.as_deref(), Some("judge_citation"));
    }

    // =============================================================
    // parse_json_loose failure modes — tested indirectly via a mock
    // LlmClient that returns malformed JSON.
    // =============================================================

    #[tokio::test]
    async fn extract_claims_propagates_parse_error() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final("not json at all".into()));
        let svc = make_service(infra.clone());

        let err = svc
            .extract_claims("summary", "Hippocampus", None, UsageContext::default())
            .await
            .expect_err("malformed JSON must surface a parse error");
        match err {
            ServiceError::Other(m) => assert!(
                m.contains("extract_claims parse error"),
                "unexpected error message: {m}"
            ),
            other => panic!("expected ServiceError::Other, got {other:?}"),
        }

        // Usage row WAS recorded because accounting runs before JSON parsing.
        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].caller_tag.as_deref(), Some("extract_claims"));
    }

    #[tokio::test]
    async fn judge_groundedness_propagates_parse_error() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final("{ malformed".into()));
        let svc = make_service(infra.clone());

        let err = svc
            .judge_groundedness("claim", &[], None, UsageContext::default())
            .await
            .expect_err("malformed JSON must surface a parse error");
        match err {
            ServiceError::Other(m) => {
                assert!(m.contains("judge_groundedness parse error"), "{m}")
            }
            other => panic!("expected ServiceError::Other, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn judge_rubric_propagates_parse_error() {
        let infra = Arc::new(MockInfra::new());
        // Valid JSON but wrong shape for RubricScores.
        infra.enqueue_summarize_ok(LlmResponse::Final(r#"{"foo": 1}"#.into()));
        let svc = make_service(infra.clone());

        let err = svc
            .judge_rubric("summary", "Hippocampus", None, UsageContext::default())
            .await
            .expect_err("schema-mismatch JSON must surface a parse error");
        match err {
            ServiceError::Other(m) => {
                assert!(m.contains("judge_rubric parse error"), "{m}")
            }
            other => panic!("expected ServiceError::Other, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn structured_chat_rejects_tool_calls() {
        // When an LLM returns tool_calls during a structured-chat call, the
        // service surfaces `ServiceError::Other`.
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::ToolCalls(vec![domain::ToolCall {
            id: "x".into(),
            name: "noop".into(),
            arguments: "{}".into(),
        }]));
        let svc = make_service(infra.clone());

        let err = svc
            .extract_claims("summary", "Hippocampus", None, UsageContext::default())
            .await
            .expect_err("tool-call response is rejected by structured_chat");
        match err {
            ServiceError::Other(m) => assert!(
                m.contains("tool calls instead of final text"),
                "unexpected error: {m}"
            ),
            other => panic!("expected ServiceError::Other, got {other:?}"),
        }
    }

    // =============================================================
    // LLM client error propagation: service bubbles InfraError up, and
    // crucially does NOT record a usage row (the accountant is only invoked
    // on the `Ok` path).
    // =============================================================

    #[tokio::test]
    async fn summarize_with_tools_llm_error_propagates_no_usage_row() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_err("upstream blew up");
        let svc = make_service(infra.clone());

        let err = svc
            .summarize_with_tools(&[], &[], None, UsageContext::default())
            .await
            .expect_err("LLM error must propagate");
        match err {
            ServiceError::InfraError(e) => assert!(e.to_string().contains("upstream blew up")),
            other => panic!("expected InfraError, got {other:?}"),
        }

        // No `llm_call_usage` row recorded on error path — `CostAccountant`
        // `finish()` short-circuits on the incoming Err.
        assert!(infra.take_records().is_empty());
    }

    #[tokio::test]
    async fn generate_queries_llm_error_propagates_no_usage_row() {
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_generate_err("rate limit");
        let svc = make_service(infra.clone());

        let err = svc
            .generate_queries("region", 3, UsageContext::default())
            .await
            .expect_err("LLM error must propagate");
        match err {
            ServiceError::InfraError(e) => assert!(e.to_string().contains("rate limit")),
            other => panic!("expected InfraError, got {other:?}"),
        }
        assert!(infra.take_records().is_empty());
    }

    // =============================================================
    // parse_json_loose direct coverage — the helper strips ```json fences
    // and falls back to the outermost balanced braces.
    // =============================================================

    #[tokio::test]
    async fn extract_claims_tolerates_markdown_fenced_json() {
        let infra = Arc::new(MockInfra::new());
        let fenced = format!("```json\n{}\n```", well_formed_claims_json());
        infra.enqueue_summarize_ok(LlmResponse::Final(fenced));
        let svc = make_service(infra.clone());

        let resp = svc
            .extract_claims("summary", "Hippocampus", None, UsageContext::default())
            .await
            .expect("fenced JSON parses after fence-stripping");
        assert_eq!(resp.claims.len(), 1);
    }

    // =============================================================
    // Provider routing: base_url + api_key forwarded to infra based
    // on `resolve_llm_provider` precedence.
    // =============================================================

    #[tokio::test]
    async fn summarize_routes_to_openrouter_by_default() {
        // Only OPENROUTER_API_KEY is set in MockInfra::new().
        let infra = Arc::new(MockInfra::new());
        infra.enqueue_summarize_ok(LlmResponse::Final("ok".into()));
        let svc = make_service(infra.clone());

        svc.summarize_with_tools(&[], &[], None, UsageContext::default())
            .await
            .unwrap();

        let calls = infra.take_llm_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://openrouter.ai/api/v1");
        assert_eq!(calls[0].1, "sk-test");
    }

    #[tokio::test]
    async fn summarize_routes_to_requesty_when_key_set() {
        let mut infra = MockInfra::new();
        infra
            .env
            .insert("REQUESTY_API_KEY".to_string(), "req-key".to_string());
        let infra = Arc::new(infra);
        infra.enqueue_summarize_ok(LlmResponse::Final("ok".into()));
        let svc = make_service(infra.clone());

        svc.summarize_with_tools(&[], &[], None, UsageContext::default())
            .await
            .unwrap();

        let calls = infra.take_llm_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0, "https://router.requesty.ai/v1",
            "Requesty default base URL"
        );
        assert_eq!(calls[0].1, "req-key", "Requesty api key forwarded");
    }

    #[tokio::test]
    async fn summarize_honours_explicit_requesty_base_url_override() {
        let mut infra = MockInfra::new();
        infra
            .env
            .insert("REQUESTY_API_KEY".to_string(), "req-key".to_string());
        infra.env.insert(
            "REQUESTY_BASE_URL".to_string(),
            "https://custom.requesty.example/v1".to_string(),
        );
        let infra = Arc::new(infra);
        infra.enqueue_summarize_ok(LlmResponse::Final("ok".into()));
        let svc = make_service(infra.clone());

        svc.summarize_with_tools(&[], &[], None, UsageContext::default())
            .await
            .unwrap();

        let calls = infra.take_llm_calls();
        assert_eq!(calls[0].0, "https://custom.requesty.example/v1");
    }

    #[tokio::test]
    async fn generate_queries_routes_to_requesty_when_key_set() {
        let mut infra = MockInfra::new();
        infra
            .env
            .insert("REQUESTY_API_KEY".to_string(), "req-key".to_string());
        let infra = Arc::new(infra);
        infra.enqueue_generate_ok(vec!["q".into()]);
        let svc = make_service(infra.clone());

        svc.generate_queries("hippocampus", 1, UsageContext::default())
            .await
            .unwrap();

        let calls = infra.take_llm_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://router.requesty.ai/v1");
        assert_eq!(calls[0].1, "req-key");
    }
}
