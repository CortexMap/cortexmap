//! Handler-level axum tests for brainatlas-be (Task 3.3).
//!
//! These tests exercise the real `BrainAtlasServer::into_router()` against a
//! hand-rolled in-memory `Services` fake (no DB, no HTTP). The fake records
//! every LLM/embed/usage call so we can assert the handler plumbing
//! (query-string parsing, JSON deserialisation, error mapping, route table)
//! without depending on infra.
//!
//! Pattern mirrors `evals-be/crates/app/tests/cache_hit.rs` and the
//! `MockInfra` in `brainatlas-be/crates/services/src/cost_accounting.rs`
//! (hand-rolled `Mutex<Vec<…>>` fakes, no mockall / no wiremock).
//!
//! Run with: `cargo test --test handler_test -- --test-threads=1`

use api::BrainAtlasApi;
use app::{
    BrainRegionInfo, Chunker, EmbeddingService, ListBrainRegions, LlmService, S3Storage,
    UsageQuery, VectorDatabase,
};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use domain::{
    BrainRegionEntry, ChunkSource, Claim, ClaimsResponse, ExistingSummary, GroundednessLabel,
    GroundednessVerdict, LlmResponse, NewEmbedding, NewRegionSummary, RegionMapping,
    RubricCriterion, RubricScores, SimilarChunk, UsageAggregate, UsageAggregateFilter,
    UsageByCallerTag, UsageByModel, UsageContext,
};
use http_body_util::BodyExt;
use server::BrainAtlasServer;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
#[error("fake error: {0}")]
struct FakeErr(String);

/// Canned responses/recorded calls for the fake `Services` impl.
#[derive(Default)]
struct FakeState {
    /// Last `UsageAggregateFilter` received by `usage_aggregate`.
    last_usage_filter: Mutex<Option<UsageAggregateFilter>>,
    /// Canned aggregate response returned from `usage_aggregate`.
    usage_response: Mutex<UsageAggregate>,
    /// Recorded embed calls: `(text, model_override, correlation_id)`.
    embed_calls: Mutex<Vec<(String, Option<String>, Option<String>)>>,
    /// Recorded extract_claims calls: `(summary_text, region_name, model, correlation_id)`.
    extract_claims_calls:
        Mutex<Vec<(String, String, Option<String>, Option<String>)>>,
    /// Recorded generate_queries calls: `(region_name, count, correlation_id)`.
    generate_queries_calls: Mutex<Vec<(String, u32, Option<String>)>>,
    /// Recorded judge_groundedness calls.
    judge_groundedness_calls:
        Mutex<Vec<(String, Vec<String>, Option<String>, Option<String>)>>,
    /// Recorded judge_rubric calls.
    judge_rubric_calls: Mutex<Vec<(String, String, Option<String>, Option<String>)>>,
    /// Recorded judge_citation calls.
    judge_citation_calls:
        Mutex<Vec<(String, String, String, Option<String>, Option<String>)>>,
    /// Regions returned by `list()`.
    regions: Mutex<Vec<RegionMapping>>,
    /// If true, `list()` returns an error (used for 500 paths).
    list_fails: Mutex<bool>,
}

struct FakeServices {
    state: Arc<FakeState>,
}

impl FakeServices {
    fn new(state: Arc<FakeState>) -> Self {
        Self { state }
    }
}

impl Chunker for FakeServices {
    fn chunk(&self, _text: &str, _chunk_size: usize, _overlap: usize) -> Vec<String> {
        Vec::new()
    }
}

#[async_trait]
impl ListBrainRegions for FakeServices {
    type Error = FakeErr;
    async fn list(&self) -> Result<Vec<RegionMapping>, Self::Error> {
        if *self.state.list_fails.lock().unwrap() {
            return Err(FakeErr("list boom".to_string()));
        }
        Ok(self.state.regions.lock().unwrap().clone())
    }
}

#[async_trait]
impl BrainRegionInfo for FakeServices {
    type Error = FakeErr;
    async fn search(&self, _id: Uuid) -> Result<Vec<BrainRegionEntry>, Self::Error> {
        Ok(vec![BrainRegionEntry::new(
            1,
            "Hippocampus".to_string(),
            "HPC".to_string(),
            "summary".to_string(),
        )])
    }
}

#[async_trait]
impl LlmService for FakeServices {
    type Error = FakeErr;

    async fn summarize_with_tools(
        &self,
        _messages: &[serde_json::Value],
        _tools: &[serde_json::Value],
        _chat_model_override: Option<&str>,
        _ctx: UsageContext,
    ) -> Result<LlmResponse, Self::Error> {
        Ok(LlmResponse::Final("summary".to_string()))
    }

    async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
        ctx: UsageContext,
    ) -> Result<Vec<String>, Self::Error> {
        self.state.generate_queries_calls.lock().unwrap().push((
            region_name.to_string(),
            count,
            ctx.correlation_id.clone(),
        ));
        Ok((0..count).map(|i| format!("q{}", i)).collect())
    }

    async fn extract_claims(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<ClaimsResponse, Self::Error> {
        self.state.extract_claims_calls.lock().unwrap().push((
            summary_text.to_string(),
            region_name.to_string(),
            chat_model_override.map(|s| s.to_string()),
            ctx.correlation_id.clone(),
        ));
        Ok(ClaimsResponse {
            claims: vec![Claim {
                id: 1,
                section: "Overview".to_string(),
                text: "fake claim".to_string(),
                cited_chunks: vec![],
            }],
        })
    }

    async fn judge_groundedness(
        &self,
        claim_text: &str,
        evidence_chunks: &[String],
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<GroundednessVerdict, Self::Error> {
        self.state.judge_groundedness_calls.lock().unwrap().push((
            claim_text.to_string(),
            evidence_chunks.to_vec(),
            chat_model_override.map(|s| s.to_string()),
            ctx.correlation_id.clone(),
        ));
        Ok(GroundednessVerdict {
            verdict: GroundednessLabel::Supported,
            confidence: 0.9,
            supporting_chunks: vec![1],
            rationale: "ok".to_string(),
        })
    }

    async fn judge_rubric(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<RubricScores, Self::Error> {
        self.state.judge_rubric_calls.lock().unwrap().push((
            summary_text.to_string(),
            region_name.to_string(),
            chat_model_override.map(|s| s.to_string()),
            ctx.correlation_id.clone(),
        ));
        Ok(rubric_fixture())
    }

    async fn judge_citation(
        &self,
        claim_text: &str,
        sentence_context: &str,
        chunk_text: &str,
        chat_model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<GroundednessVerdict, Self::Error> {
        self.state.judge_citation_calls.lock().unwrap().push((
            claim_text.to_string(),
            sentence_context.to_string(),
            chunk_text.to_string(),
            chat_model_override.map(|s| s.to_string()),
            ctx.correlation_id.clone(),
        ));
        Ok(GroundednessVerdict {
            verdict: GroundednessLabel::Partial,
            confidence: 0.5,
            supporting_chunks: vec![],
            rationale: "partial".to_string(),
        })
    }
}

#[async_trait]
impl EmbeddingService for FakeServices {
    type Error = FakeErr;
    async fn generate_embedding(
        &self,
        text: &str,
        model_override: Option<&str>,
        ctx: UsageContext,
    ) -> Result<Vec<f32>, Self::Error> {
        self.state.embed_calls.lock().unwrap().push((
            text.to_string(),
            model_override.map(|s| s.to_string()),
            ctx.correlation_id.clone(),
        ));
        Ok(vec![0.1_f32, 0.2, 0.3])
    }
}

#[async_trait]
impl S3Storage for FakeServices {
    type Error = FakeErr;
    async fn download(&self, _key: &str) -> Result<String, Self::Error> {
        Ok(String::new())
    }
}

#[async_trait]
impl VectorDatabase for FakeServices {
    type Error = FakeErr;
    async fn check_content_hash(
        &self,
        _region_id: i32,
        _content_hash: &str,
    ) -> Result<Option<ExistingSummary>, Self::Error> {
        Ok(None)
    }
    async fn insert_summary_with_embeddings(
        &self,
        _summary: NewRegionSummary,
        _embeddings: Vec<NewEmbedding>,
    ) -> Result<Uuid, Self::Error> {
        Ok(Uuid::nil())
    }
    async fn search_similar(
        &self,
        _query_embedding: Vec<f32>,
        _region_id: i32,
        _top_k: usize,
    ) -> Result<Vec<SimilarChunk>, Self::Error> {
        Ok(Vec::new())
    }
    async fn update_summary_text(
        &self,
        _summary_id: Uuid,
        _summary_text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    async fn get_chunk_source(
        &self,
        _chunk_id: Uuid,
    ) -> Result<Option<ChunkSource>, Self::Error> {
        Ok(None)
    }
}

#[async_trait]
impl UsageQuery for FakeServices {
    type Error = FakeErr;
    async fn usage_aggregate(
        &self,
        filter: UsageAggregateFilter,
    ) -> Result<UsageAggregate, Self::Error> {
        *self.state.last_usage_filter.lock().unwrap() = Some(filter);
        Ok(self.state.usage_response.lock().unwrap().clone())
    }
}

fn rubric_fixture() -> RubricScores {
    let criterion = |score: u8, rationale: &str| RubricCriterion {
        score,
        rationale: rationale.to_string(),
    };
    RubricScores {
        relevance: criterion(5, "relevance"),
        coherence: criterion(4, "coherence"),
        specificity: criterion(3, "specificity"),
        clinical_utility: criterion(5, "utility"),
        terminology: criterion(4, "terminology"),
    }
}

/// Build the real axum router backed by a `FakeServices` instance.
fn build_app(state: Arc<FakeState>) -> Router {
    let services = Arc::new(FakeServices::new(state));
    let api = Arc::new(BrainAtlasApi::new(services));
    let server = BrainAtlasServer::new(api);
    server.into_router(None)
}

async fn read_body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    if bytes.is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

// ------------------------------------------------------------------
// `/api/llm/usage` — every query-string filter maps into
// `UsageAggregateFilter` correctly.
// ------------------------------------------------------------------

#[tokio::test]
async fn usage_endpoint_propagates_every_filter() {
    let state = Arc::new(FakeState::default());
    // Seed a non-default aggregate so we can assert the handler actually
    // returned the repo's reply byte-for-byte.
    *state.usage_response.lock().unwrap() = UsageAggregate {
        total_cost_usd: 1.25,
        total_tokens: 100,
        total_prompt_tokens: 60,
        total_completion_tokens: 40,
        total_calls: 3,
        by_model: vec![UsageByModel {
            model: "openai/gpt-4o-mini".to_string(),
            total_cost_usd: 1.25,
            total_tokens: 100,
            total_calls: 3,
        }],
        by_caller_tag: vec![UsageByCallerTag {
            caller_tag: "orch".to_string(),
            total_cost_usd: 1.25,
            total_tokens: 100,
            total_calls: 3,
        }],
    };
    let app = build_app(state.clone());

    let summary_id = "11111111-1111-1111-1111-111111111111";
    let batch_id = "22222222-2222-2222-2222-222222222222";
    let uri = format!(
        "/brainatlas-be/api/llm/usage?\
         since=2026-04-01T00:00:00Z&\
         until=2026-04-20T23:59:59Z&\
         model=openai%2Fgpt-4o-mini&\
         correlation_id=eval%3Arun-1%3Astep-2&\
         correlation_id_prefix=eval%3Arun-1%3A&\
         region_id=42&\
         summary_id={summary_id}&\
         batch_id={batch_id}&\
         caller_tag=orch"
    );
    let req = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert!((body["total_cost_usd"].as_f64().unwrap() - 1.25).abs() < 1e-9);
    assert_eq!(body["total_tokens"], 100);
    assert_eq!(body["by_model"][0]["model"], "openai/gpt-4o-mini");
    assert_eq!(body["by_caller_tag"][0]["caller_tag"], "orch");

    let captured = state.last_usage_filter.lock().unwrap().clone().unwrap();
    assert_eq!(
        captured.since.unwrap().to_rfc3339(),
        "2026-04-01T00:00:00+00:00"
    );
    assert_eq!(
        captured.until.unwrap().to_rfc3339(),
        "2026-04-20T23:59:59+00:00"
    );
    assert_eq!(captured.model.as_deref(), Some("openai/gpt-4o-mini"));
    assert_eq!(captured.correlation_id.as_deref(), Some("eval:run-1:step-2"));
    assert_eq!(
        captured.correlation_id_prefix.as_deref(),
        Some("eval:run-1:")
    );
    assert_eq!(captured.region_id, Some(42));
    assert_eq!(captured.summary_id.unwrap().to_string(), summary_id);
    assert_eq!(captured.batch_id.unwrap().to_string(), batch_id);
    assert_eq!(captured.caller_tag.as_deref(), Some("orch"));
}

#[tokio::test]
async fn usage_endpoint_empty_query_string_yields_default_filter() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let req = Request::builder()
        .method(Method::GET)
        .uri("/brainatlas-be/api/llm/usage")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let filter = state.last_usage_filter.lock().unwrap().clone().unwrap();
    assert!(filter.since.is_none());
    assert!(filter.until.is_none());
    assert!(filter.model.is_none());
    assert!(filter.correlation_id.is_none());
    assert!(filter.correlation_id_prefix.is_none());
    assert!(filter.region_id.is_none());
    assert!(filter.summary_id.is_none());
    assert!(filter.batch_id.is_none());
    assert!(filter.caller_tag.is_none());
}

#[tokio::test]
async fn usage_endpoint_rejects_malformed_timestamp() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let req = Request::builder()
        .method(Method::GET)
        .uri("/brainatlas-be/api/llm/usage?since=not-a-date")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    // The repo must not have been called.
    assert!(state.last_usage_filter.lock().unwrap().is_none());
}

#[tokio::test]
async fn usage_endpoint_rejects_malformed_summary_uuid() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let req = Request::builder()
        .method(Method::GET)
        .uri("/brainatlas-be/api/llm/usage?summary_id=not-a-uuid")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(state.last_usage_filter.lock().unwrap().is_none());
}

#[tokio::test]
async fn usage_endpoint_rejects_malformed_batch_uuid() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let req = Request::builder()
        .method(Method::GET)
        .uri("/brainatlas-be/api/llm/usage?batch_id=xxx")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn usage_endpoint_region_id_is_numeric() {
    // `region_id` on the query wire is `i32`, so non-numeric values must be
    // rejected by the axum `Query` extractor (400) rather than reaching the
    // repo.
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let req = Request::builder()
        .method(Method::GET)
        .uri("/brainatlas-be/api/llm/usage?region_id=not-an-int")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(state.last_usage_filter.lock().unwrap().is_none());
}

// ------------------------------------------------------------------
// Eval-orchestration manual trigger endpoints: `/api/llm/extract-claims`,
// `/api/llm/judge-groundedness`, `/api/llm/judge-rubric`,
// `/api/llm/judge-citation`, `/api/llm/embed`.
// ------------------------------------------------------------------

#[tokio::test]
async fn extract_claims_endpoint_round_trips_body_and_records_call() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let body = serde_json::json!({
        "summary_text": "## Overview\nThe hippocampus matters.",
        "region_name": "Hippocampus",
        "chat_model": "openai/gpt-4o-mini",
        "correlation_id": "eval:run-1:step-1"
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/llm/extract-claims")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let reply = read_body_json(resp).await;
    assert_eq!(reply["claims"][0]["text"], "fake claim");
    assert_eq!(reply["claims"][0]["id"], 1);

    let calls = state.extract_claims_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "## Overview\nThe hippocampus matters.");
    assert_eq!(calls[0].1, "Hippocampus");
    assert_eq!(calls[0].2.as_deref(), Some("openai/gpt-4o-mini"));
    assert_eq!(calls[0].3.as_deref(), Some("eval:run-1:step-1"));
}

#[tokio::test]
async fn judge_groundedness_endpoint_records_correlation_id() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let body = serde_json::json!({
        "claim_text": "The hippocampus supports memory.",
        "evidence_chunks": ["chunk one", "chunk two"],
        "correlation_id": "eval:run-2:step-5"
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/llm/judge-groundedness")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let reply = read_body_json(resp).await;
    assert_eq!(reply["verdict"], "supported");
    assert!((reply["confidence"].as_f64().unwrap() - 0.9).abs() < 1e-6);

    let calls = state.judge_groundedness_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "The hippocampus supports memory.");
    assert_eq!(calls[0].1, vec!["chunk one".to_string(), "chunk two".to_string()]);
    assert!(calls[0].2.is_none()); // chat_model omitted
    assert_eq!(calls[0].3.as_deref(), Some("eval:run-2:step-5"));
}

#[tokio::test]
async fn judge_rubric_endpoint_returns_five_criteria() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let body = serde_json::json!({
        "summary_text": "A summary.",
        "region_name": "CTX"
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/llm/judge-rubric")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let reply = read_body_json(resp).await;
    assert_eq!(reply["relevance"]["score"], 5);
    assert_eq!(reply["coherence"]["score"], 4);
    assert_eq!(reply["specificity"]["score"], 3);
    assert_eq!(reply["clinical_utility"]["score"], 5);
    assert_eq!(reply["terminology"]["score"], 4);

    let calls = state.judge_rubric_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "A summary.");
    assert_eq!(calls[0].1, "CTX");
}

#[tokio::test]
async fn judge_citation_endpoint_records_three_fields() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let body = serde_json::json!({
        "claim_text": "Hippocampus supports memory.",
        "sentence_context": "The hippocampus supports memory [chunk:abc].",
        "chunk_text": "Hippocampal lesions impair declarative memory.",
        "chat_model": "openai/gpt-4o-mini",
        "correlation_id": "eval:run-3:step-9"
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/llm/judge-citation")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let reply = read_body_json(resp).await;
    assert_eq!(reply["verdict"], "partial");

    let calls = state.judge_citation_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "Hippocampus supports memory.");
    assert_eq!(
        calls[0].1,
        "The hippocampus supports memory [chunk:abc]."
    );
    assert_eq!(calls[0].2, "Hippocampal lesions impair declarative memory.");
    assert_eq!(calls[0].3.as_deref(), Some("openai/gpt-4o-mini"));
    assert_eq!(calls[0].4.as_deref(), Some("eval:run-3:step-9"));
}

#[tokio::test]
async fn embed_endpoint_returns_vector_and_forwards_model_override() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let body = serde_json::json!({
        "text": "hippocampus memory",
        "embedding_model": "text-embedding-3-small",
        "correlation_id": "eval:run-4:step-1"
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/llm/embed")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let reply = read_body_json(resp).await;
    let embedding = reply["embedding"].as_array().unwrap();
    assert_eq!(embedding.len(), 3);

    let calls = state.embed_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "hippocampus memory");
    assert_eq!(calls[0].1.as_deref(), Some("text-embedding-3-small"));
    assert_eq!(calls[0].2.as_deref(), Some("eval:run-4:step-1"));
}

// ------------------------------------------------------------------
// Malformed / unauthorized request paths.
// ------------------------------------------------------------------

#[tokio::test]
async fn malformed_json_body_returns_client_error() {
    // axum returns 400 for invalid JSON via the `Json` extractor.
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/llm/embed")
        .header("content-type", "application/json")
        .body(Body::from("not json"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // Any 4xx is acceptable — axum returns 400 Bad Request for malformed JSON
    // and 422 Unprocessable Entity when the body is syntactically valid but
    // semantically wrong. Either way, the fake must not have been called.
    assert!(resp.status().is_client_error(), "got {}", resp.status());
    assert!(state.embed_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/brainatlas-be/api/not-a-real-endpoint")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn wrong_method_returns_405() {
    // GET-only route hit with POST should return 405 Method Not Allowed.
    let state = Arc::new(FakeState::default());
    let app = build_app(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/list")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn status_endpoint_missing_id_returns_400() {
    // The /api/status handler returns MissingOrInvalidId (400) when the body
    // lacks a parseable UUID. Contract with orch.
    let state = Arc::new(FakeState::default());
    let app = build_app(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/status")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn process_endpoint_missing_region_id_returns_400() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state);

    // Missing both region_id and batch_id — handler layer rejects with 400
    // via `ApiError::MissingOrInvalidId`.
    let body = serde_json::json!({
        "s3_keys": [],
        "paper_metadata": []
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/process")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ------------------------------------------------------------------
// Sanity: generate_queries + list + health.
// ------------------------------------------------------------------

#[tokio::test]
async fn generate_queries_endpoint_returns_canned_list() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state.clone());

    let body = serde_json::json!({
        "region_name": "hippocampus",
        "count": 3u32,
        "correlation_id": "region:42"
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/brainatlas-be/api/generate-queries")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let reply = read_body_json(resp).await;
    let queries = reply["queries"].as_array().unwrap();
    assert_eq!(queries.len(), 3);

    let calls = state.generate_queries_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "hippocampus");
    assert_eq!(calls[0].1, 3);
    assert_eq!(calls[0].2.as_deref(), Some("region:42"));
}

#[tokio::test]
async fn list_endpoint_surfaces_service_errors_as_500() {
    let state = Arc::new(FakeState::default());
    *state.list_fails.lock().unwrap() = true;
    let app = build_app(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/brainatlas-be/api/list")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn health_endpoint_is_reachable() {
    let state = Arc::new(FakeState::default());
    let app = build_app(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/brainatlas-be/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["status"], "ok");
}
