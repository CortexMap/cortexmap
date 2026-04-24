//! Wire-protocol contract test — orch <-> evals-be.
//!
//! The highest-value single test in PR #69. See plan Task 3.5:
//! `plans/2026-04-20-pr69-max-test-coverage-v1.md`.
//!
//! ## What this test guards
//!
//! orch and evals-be are two independent services. Their coupling is a
//! single JSON-over-HTTP wire protocol:
//!
//!   * orch calls `POST /evals-be/api/evals/score/init` with an
//!     `InitScoreRequest` and receives an `InitScoreResponse` whose `next`
//!     field is a `NextAction::CallLlm { step_id, endpoint, path, body }`.
//!   * For each `CallLlm`, orch POSTs `body` to `{brainatlas}{path}` and
//!     receives an LLM response.
//!   * orch wraps the LLM response in an `LlmResponsePayload` variant
//!     matching the `endpoint`, POSTs it back to
//!     `POST /evals-be/api/evals/score/step` inside a `StepRequest`, and
//!     reads the next `StepResponse`.
//!   * The loop terminates when `NextAction::Done { metrics }` arrives.
//!
//! If the JSON shapes drift on either side the two services stop talking.
//! This test instantiates BOTH halves in a single process:
//!
//!   * The evals-be axum `Router` is built from scratch around the generic
//!     `Evals<InMemoryDb, DummyEnv, MockError>` state (see `build_router`).
//!   * A `FakeHttpClient` wraps that Router and routes orch's outbound
//!     requests by path. Evals-be paths are dispatched via
//!     `tower::ServiceExt::oneshot`; brainatlas LLM paths are served from a
//!     handful of deterministic canned responses.
//!   * A `drive_one` function (a byte-accurate mirror of orch's private
//!     `drive_one` at `orch/crates/services/src/eval_orchestrator.rs:404`)
//!     drives the full loop using ONLY `evals-rpc-types` — the canonical
//!     wire types — on both ends.
//!
//! ## Why this lives outside every service workspace
//!
//! We originally tried to add `evals-app` as a dev-dep of
//! `orch/crates/services`. That fails because both `orch/crates/domain` and
//! `brainatlas-be/crates/domain` register themselves to Cargo as the package
//! named `domain`, and `evals-app` depends (transitively) on the
//! brainatlas-be one. Cargo refuses to have two different `domain`
//! packages in the same resolve graph. Living outside both workspaces
//! sidesteps the collision: we only pull in evals-be crates (which use the
//! rename `brainatlas-domain = { package = "domain", ... }`) and never
//! touch orch's package graph.
//!
//! The orch side is reconstituted by mirroring the private `drive_one`
//! function. Since orch's private wire-type mirrors are themselves derived
//! from `evals-rpc-types`, any drift in orch's side will break orch's own
//! unit tests; any drift in the HTTP shape on either side will break this
//! test.

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, extract::Json as ExtractJson};
use bytes::Bytes;
use chrono::NaiveDateTime;
use evals_api::{ApiError, Evals, EvalsApi};
use evals_app::{AppError, EvalRuntimeConfig, EvalsApp};
use evals_domain::{
    Claim, ClaimsResponse, EvalRun, EvalRunStatus, EvalScore, GroundednessLabel,
    GroundednessVerdict, NewEvalScore, RubricCriterion, RubricScores,
};
use evals_rpc_types::{
    InitScoreRequest, InitScoreResponse, LlmEndpoint, LlmResponsePayload, NextAction, StepRequest,
    StepResponse,
};
use evals_services::{
    ChunkRow, EnvInfra, EvalAggregate, EvalsDatabase, RetrievedChunk, SummaryRow,
    WorstOffenderRow,
};
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use uuid::Uuid;

// =============================================================================
// Section 1. Errors and in-memory EvalsDatabase + EnvInfra implementations.
//            These mirror the fakes from
//            `evals-be/crates/app/tests/cache_hit.rs:32-294`.
// =============================================================================

#[derive(Debug, thiserror::Error)]
#[error("mock infra error: {0}")]
struct MockError(String);

#[derive(Default)]
struct InMemoryDb {
    summary: Mutex<Option<SummaryRow>>,
    scores: Mutex<Vec<EvalScore>>,
    runs: Mutex<Vec<EvalRun>>,
    run_states: Mutex<Vec<RunStateRow>>,
    unscored: Mutex<Vec<Uuid>>,
}

#[derive(Clone)]
struct RunStateRow {
    run_id: Uuid,
    summary_id: Uuid,
    eval_version: String,
    state: serde_json::Value,
    pending_step_id: Option<Uuid>,
}

impl InMemoryDb {
    fn new(summary: SummaryRow, unscored: Vec<Uuid>) -> Self {
        Self {
            summary: Mutex::new(Some(summary)),
            scores: Mutex::new(Vec::new()),
            runs: Mutex::new(Vec::new()),
            run_states: Mutex::new(Vec::new()),
            unscored: Mutex::new(unscored),
        }
    }
}

#[async_trait]
impl EvalsDatabase for InMemoryDb {
    type Error = MockError;

    async fn lookup_score_by_hash(
        &self,
        _database_url: &str,
        summary_hash: &str,
        metric: &str,
        eval_version: &str,
    ) -> Result<Option<EvalScore>, Self::Error> {
        let scores = self.scores.lock().unwrap();
        Ok(scores
            .iter()
            .find(|s| {
                s.summary_hash == summary_hash
                    && s.metric == metric
                    && s.eval_version == eval_version
            })
            .cloned())
    }

    async fn insert_score(
        &self,
        _database_url: &str,
        new: NewEvalScore,
    ) -> Result<EvalScore, Self::Error> {
        let mut scores = self.scores.lock().unwrap();
        if let Some(existing) = scores.iter().find(|s| {
            s.summary_hash == new.summary_hash
                && s.metric == new.metric
                && s.eval_version == new.eval_version
        }) {
            return Ok(existing.clone());
        }
        let row = EvalScore {
            id: Uuid::new_v4(),
            summary_id: new.summary_id,
            summary_hash: new.summary_hash,
            metric: new.metric,
            score: new.score,
            judge_model: new.judge_model,
            details: new.details,
            eval_version: new.eval_version,
            created_at: NaiveDateTime::default(),
        };
        scores.push(row.clone());
        Ok(row)
    }

    async fn get_summary(
        &self,
        _database_url: &str,
        summary_id: Uuid,
    ) -> Result<Option<SummaryRow>, Self::Error> {
        let s = self.summary.lock().unwrap();
        Ok(s.as_ref().filter(|r| r.id == summary_id).cloned())
    }

    async fn get_scores_for_summary(
        &self,
        _database_url: &str,
        summary_id: Uuid,
    ) -> Result<Vec<EvalScore>, Self::Error> {
        Ok(self
            .scores
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.summary_id == summary_id)
            .cloned()
            .collect())
    }

    async fn get_eval_aggregate(
        &self,
        _database_url: &str,
        _eval_version: &str,
    ) -> Result<EvalAggregate, Self::Error> {
        Ok(EvalAggregate::default())
    }

    async fn get_worst_offenders(
        &self,
        _database_url: &str,
        _metric: &str,
        _eval_version: &str,
        _limit: i64,
    ) -> Result<Vec<WorstOffenderRow>, Self::Error> {
        Ok(Vec::new())
    }

    async fn upsert_run(
        &self,
        _database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
        status: EvalRunStatus,
        error_message: Option<String>,
    ) -> Result<EvalRun, Self::Error> {
        let row = EvalRun {
            id: Uuid::new_v4(),
            summary_id,
            eval_version: eval_version.to_string(),
            status,
            error_message,
            started_at: None,
            completed_at: None,
            created_at: NaiveDateTime::default(),
        };
        self.runs.lock().unwrap().push(row.clone());
        Ok(row)
    }

    async fn list_unscored_summary_ids(
        &self,
        _database_url: &str,
        _eval_version: &str,
        _limit: i64,
    ) -> Result<Vec<Uuid>, Self::Error> {
        Ok(self.unscored.lock().unwrap().clone())
    }

    async fn retrieve_chunks_for_summary(
        &self,
        _database_url: &str,
        _summary_id: Uuid,
        _embedding: &[f32],
        _top_k: i64,
        _min_similarity: f32,
    ) -> Result<Vec<RetrievedChunk>, Self::Error> {
        Ok(vec![RetrievedChunk {
            chunk_index: 1,
            chunk_text: "supporting evidence".to_string(),
            similarity: 0.9,
        }])
    }

    async fn load_chunks_by_ids(
        &self,
        _database_url: &str,
        _chunk_ids: &[Uuid],
    ) -> Result<Vec<ChunkRow>, Self::Error> {
        Ok(Vec::new())
    }

    async fn insert_run_state(
        &self,
        _database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
        state: &serde_json::Value,
        pending_step_id: Option<Uuid>,
        _pending_endpoint: Option<&str>,
    ) -> Result<Uuid, Self::Error> {
        let run_id = Uuid::new_v4();
        self.run_states.lock().unwrap().push(RunStateRow {
            run_id,
            summary_id,
            eval_version: eval_version.to_string(),
            state: state.clone(),
            pending_step_id,
        });
        Ok(run_id)
    }

    async fn load_run_state(
        &self,
        _database_url: &str,
        run_id: Uuid,
    ) -> Result<Option<(Uuid, String, serde_json::Value, Option<Uuid>)>, Self::Error> {
        let states = self.run_states.lock().unwrap();
        Ok(states.iter().find(|r| r.run_id == run_id).map(|r| {
            (
                r.summary_id,
                r.eval_version.clone(),
                r.state.clone(),
                r.pending_step_id,
            )
        }))
    }

    async fn save_run_state(
        &self,
        _database_url: &str,
        run_id: Uuid,
        state: &serde_json::Value,
        pending_step_id: Option<Uuid>,
        _pending_endpoint: Option<&str>,
    ) -> Result<(), Self::Error> {
        let mut states = self.run_states.lock().unwrap();
        if let Some(r) = states.iter_mut().find(|r| r.run_id == run_id) {
            r.state = state.clone();
            r.pending_step_id = pending_step_id;
        }
        Ok(())
    }

    async fn delete_run_state(
        &self,
        _database_url: &str,
        run_id: Uuid,
    ) -> Result<(), Self::Error> {
        let mut states = self.run_states.lock().unwrap();
        states.retain(|r| r.run_id != run_id);
        Ok(())
    }

    async fn delete_run_states_for_summary(
        &self,
        _database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
    ) -> Result<(), Self::Error> {
        let mut states = self.run_states.lock().unwrap();
        states.retain(|r| !(r.summary_id == summary_id && r.eval_version == eval_version));
        Ok(())
    }
}

struct DummyEnv;

impl EnvInfra for DummyEnv {
    type Error = MockError;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
        Err(MockError(format!("no env var {}", key)))
    }
}

// =============================================================================
// Section 2. Build the evals-be axum Router with the in-memory state.
//            Mirrors `evals-be/crates/server/src/server.rs:57-77` but generic
//            over our mock infra instead of the concrete `EvalsInfra`.
// =============================================================================

type TestEvals = Evals<InMemoryDb, DummyEnv, MockError>;
type TestEvalsError = ApiError<AppError<MockError>>;

#[derive(Clone)]
struct ServerState {
    api: Arc<TestEvals>,
}

struct ServerError(TestEvalsError);

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self.0 {
            ApiError::MissingOrInvalidId => (StatusCode::BAD_REQUEST, self.0.to_string()),
            ApiError::AppError(AppError::SummaryNotFound) => {
                (StatusCode::NOT_FOUND, "summary not found".to_string())
            }
            ApiError::AppError(AppError::InvalidArg(m)) => (StatusCode::BAD_REQUEST, m.clone()),
            ApiError::AppError(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl From<TestEvalsError> for ServerError {
    fn from(e: TestEvalsError) -> Self {
        ServerError(e)
    }
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn init_score_handler(
    State(state): State<ServerState>,
    ExtractJson(req): ExtractJson<InitScoreRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = state.api.init_score(req).await?;
    Ok(Json(resp))
}

async fn step_score_handler(
    State(state): State<ServerState>,
    ExtractJson(req): ExtractJson<StepRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = state.api.step_score(req).await?;
    Ok(Json(resp))
}

async fn scores_for_summary_handler(
    State(state): State<ServerState>,
    Path(summary_id): Path<Uuid>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = state.api.scores_for_summary(summary_id).await?;
    Ok(Json(resp))
}

#[derive(Deserialize)]
struct AggregateQuery {
    eval_version: Option<String>,
}

async fn aggregate_handler(
    State(state): State<ServerState>,
    Query(q): Query<AggregateQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = state.api.aggregate_summary(q.eval_version).await?;
    Ok(Json(resp))
}

#[derive(Deserialize)]
struct WorstQuery {
    metric: String,
    #[serde(default = "default_worst_limit")]
    limit: i64,
    eval_version: Option<String>,
}

fn default_worst_limit() -> i64 {
    20
}

async fn worst_handler(
    State(state): State<ServerState>,
    Query(q): Query<WorstQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = state
        .api
        .worst_offenders(q.metric, q.limit, q.eval_version)
        .await?;
    Ok(Json(resp))
}

#[derive(Deserialize)]
struct UnscoredQuery {
    eval_version: Option<String>,
    #[serde(default = "default_unscored_limit")]
    limit: i64,
}

fn default_unscored_limit() -> i64 {
    100
}

async fn unscored_handler(
    State(state): State<ServerState>,
    Query(q): Query<UnscoredQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = state
        .api
        .list_unscored_summary_ids(q.eval_version, q.limit)
        .await?;
    Ok(Json(resp))
}

fn build_router(state: ServerState) -> Router {
    let routes = Router::new()
        .route("/health", get(health_handler))
        .route("/api/evals/score/init", post(init_score_handler))
        .route("/api/evals/score/step", post(step_score_handler))
        .route(
            "/api/evals/scores/{summary_id}",
            get(scores_for_summary_handler),
        )
        .route("/api/evals/summary", get(aggregate_handler))
        .route("/api/evals/worst", get(worst_handler))
        .route("/api/evals/unscored", get(unscored_handler))
        .with_state(state);

    Router::new().nest("/evals-be", routes)
}

// =============================================================================
// Section 3. FakeHttpClient. Wraps the axum Router in the same process and
//            routes by URL. Paths that hit `/evals-be/...` are served by the
//            real Router via `oneshot`. Brainatlas LLM paths are served by
//            canned deterministic responses (one per `LlmEndpoint` variant).
// =============================================================================

/// Call recording for post-hoc assertions about what orch actually sent over
/// the wire. Tracks (method, url, maybe-body-bytes) in order.
#[derive(Clone, Default)]
struct CallLog {
    calls: Arc<Mutex<Vec<(String, String, Option<Bytes>)>>>,
}

impl CallLog {
    fn record(&self, method: &str, url: &str, body: Option<Bytes>) {
        self.calls
            .lock()
            .unwrap()
            .push((method.to_string(), url.to_string(), body));
    }

    fn all(&self) -> Vec<(String, String, Option<Bytes>)> {
        self.calls.lock().unwrap().clone()
    }
}

struct FakeHttpClient {
    evals_router: Router,
    calls: CallLog,
}

impl FakeHttpClient {
    fn new(evals_router: Router) -> Self {
        Self {
            evals_router,
            calls: CallLog::default(),
        }
    }

    fn calls(&self) -> CallLog {
        self.calls.clone()
    }

    /// Split a URL into "base" and "path?query" where path is everything
    /// starting from the first `/` after the authority component.
    fn split_url(url: &str) -> (&str, &str) {
        let without_scheme = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .unwrap_or(url);
        if let Some(slash_idx) = without_scheme.find('/') {
            // Keep the original scheme for the base.
            let scheme_len = url.len() - without_scheme.len();
            let base_end = scheme_len + slash_idx;
            (&url[..base_end], &url[base_end..])
        } else {
            (url, "/")
        }
    }

    async fn dispatch_evals_post<Req: Serialize>(
        &self,
        path_and_query: &str,
        body: &Req,
    ) -> serde_json::Value {
        let body_bytes = Bytes::from(serde_json::to_vec(body).expect("serialize body"));
        self.calls
            .record("POST", path_and_query, Some(body_bytes.clone()));
        let req = Request::builder()
            .method("POST")
            .uri(path_and_query)
            .header("content-type", "application/json")
            .body(Body::from(body_bytes))
            .expect("build POST request");
        self.exec_router(req).await
    }

    async fn exec_router(&self, req: Request<Body>) -> serde_json::Value {
        let resp = self
            .evals_router
            .clone()
            .oneshot(req)
            .await
            .expect("router oneshot infallible");

        let status = resp.status();
        let body_bytes = resp
            .into_body()
            .collect()
            .await
            .expect("collect response body")
            .to_bytes();

        assert!(
            status.is_success(),
            "evals-be router returned non-2xx: status={} body={}",
            status,
            String::from_utf8_lossy(&body_bytes)
        );

        serde_json::from_slice(&body_bytes).unwrap_or_else(|e| {
            panic!(
                "evals-be response not valid JSON: error={e} body={}",
                String::from_utf8_lossy(&body_bytes)
            )
        })
    }

    /// Canned deterministic response for the five brainatlas LLM endpoints.
    /// Shape must match what evals-be's `LlmResponsePayload::{variant}`
    /// inner type expects, since orch wraps the raw LLM response verbatim
    /// inside the payload envelope before shipping it back on `/step`.
    fn brainatlas_response_for(endpoint: LlmEndpoint) -> serde_json::Value {
        match endpoint {
            LlmEndpoint::ExtractClaims => serde_json::to_value(ClaimsResponse {
                claims: vec![
                    Claim {
                        id: 1,
                        section: "Overview".to_string(),
                        text: "The hippocampus supports declarative memory.".to_string(),
                        cited_chunks: vec![],
                    },
                    Claim {
                        id: 2,
                        section: "Anatomy".to_string(),
                        text: "It sits in the medial temporal lobe.".to_string(),
                        cited_chunks: vec![],
                    },
                    Claim {
                        id: 3,
                        section: "Functions".to_string(),
                        text: "It is implicated in Alzheimer's disease.".to_string(),
                        cited_chunks: vec![],
                    },
                ],
            })
            .unwrap(),
            LlmEndpoint::Embed => serde_json::to_value(
                brainatlas_rpc_types::evals::EmbedResponse {
                    embedding: vec![0.1_f32; 8],
                },
            )
            .unwrap(),
            LlmEndpoint::JudgeGroundedness | LlmEndpoint::JudgeCitation => {
                serde_json::to_value(GroundednessVerdict {
                    verdict: GroundednessLabel::Supported,
                    confidence: 0.95,
                    supporting_chunks: vec![1],
                    rationale: "matches retrieved chunk".to_string(),
                })
                .unwrap()
            }
            LlmEndpoint::JudgeRubric => {
                let c = |s: u8| RubricCriterion {
                    score: s,
                    rationale: format!("score {}", s),
                };
                serde_json::to_value(RubricScores {
                    relevance: c(5),
                    coherence: c(4),
                    specificity: c(4),
                    clinical_utility: c(4),
                    terminology: c(4),
                })
                .unwrap()
            }
        }
    }
}

// =============================================================================
// Section 4. Byte-accurate mirror of orch's private `drive_one` loop at
//            `orch/crates/services/src/eval_orchestrator.rs:404-498`.
//
// Since the orch function is private AND we cannot depend on orch's crate
// graph from this workspace (see the rationale comment at the top of the
// file), we re-implement the loop here using the SAME wire types from
// `evals-rpc-types` that orch's private mirrors are copied from. If either
// side drifts from that canonical crate, this test breaks.
// =============================================================================

/// Safety bound identical to `MAX_STEPS_PER_RUN` at
/// `orch/crates/services/src/eval_orchestrator.rs:147`.
const MAX_STEPS_PER_RUN: usize = 100;

/// Drive one summary through the init -> step* -> Done wire protocol.
/// Returns `(step_count, final_metrics)` on success.
async fn drive_one(
    client: &FakeHttpClient,
    evals_base: &str,
    brainatlas_base: &str,
    summary_id: Uuid,
    eval_version: &str,
) -> Result<(usize, Vec<evals_rpc_types::MetricResult>), String> {
    let init_url = format!("{}/evals-be/api/evals/score/init", evals_base);
    let step_url = format!("{}/evals-be/api/evals/score/step", evals_base);

    let init_req = InitScoreRequest {
        summary_id,
        eval_version: Some(eval_version.to_string()),
    };

    let (_, init_path) = FakeHttpClient::split_url(&init_url);
    let init_resp_json = client.dispatch_evals_post(init_path, &init_req).await;
    let init_resp: InitScoreResponse =
        serde_json::from_value(init_resp_json).map_err(|e| format!("init decode: {e}"))?;

    assert_eq!(init_resp.summary_id, summary_id);
    assert_eq!(init_resp.eval_version, eval_version);

    let mut run_id = init_resp.run_id;
    let mut next = init_resp.next;
    let mut step_count = 0usize;

    for _ in 0..MAX_STEPS_PER_RUN {
        match next {
            NextAction::Done { metrics } => {
                return Ok((step_count, metrics));
            }
            NextAction::CallLlm {
                step_id,
                endpoint,
                path,
                body,
            } => {
                step_count += 1;

                // Orch's actual behavior: POST the body (with a correlation_id
                // injected) to brainatlas_base + path, then forward the
                // response wrapped in a typed envelope. We emulate brainatlas
                // here by returning a canned deterministic response for each
                // endpoint variant (see FakeHttpClient::brainatlas_response_for).
                let llm_url = format!("{}{}", brainatlas_base, path);
                let mut body_with_corr = body.clone();
                if let Some(obj) = body_with_corr.as_object_mut() {
                    obj.insert(
                        "correlation_id".to_string(),
                        serde_json::Value::String(format!("eval:{}:{}", run_id, step_id)),
                    );
                }
                // Record the would-be brainatlas call on the call log even
                // though the router only serves evals-be paths. This keeps
                // later assertions about correlation-id propagation possible.
                client.calls.record(
                    "POST",
                    &llm_url,
                    Some(Bytes::from(
                        serde_json::to_vec(&body_with_corr).expect("serialize llm body"),
                    )),
                );
                let llm_resp_json = FakeHttpClient::brainatlas_response_for(endpoint);

                let payload = match endpoint {
                    LlmEndpoint::ExtractClaims => LlmResponsePayload::Claims(
                        serde_json::from_value(llm_resp_json)
                            .map_err(|e| format!("claims decode: {e}"))?,
                    ),
                    LlmEndpoint::Embed => LlmResponsePayload::Embed(
                        serde_json::from_value(llm_resp_json)
                            .map_err(|e| format!("embed decode: {e}"))?,
                    ),
                    LlmEndpoint::JudgeGroundedness => LlmResponsePayload::Groundedness(
                        serde_json::from_value(llm_resp_json)
                            .map_err(|e| format!("groundedness decode: {e}"))?,
                    ),
                    LlmEndpoint::JudgeRubric => LlmResponsePayload::Rubric(
                        serde_json::from_value(llm_resp_json)
                            .map_err(|e| format!("rubric decode: {e}"))?,
                    ),
                    LlmEndpoint::JudgeCitation => LlmResponsePayload::CitationSupport(
                        serde_json::from_value(llm_resp_json)
                            .map_err(|e| format!("citation decode: {e}"))?,
                    ),
                };

                let step_req = StepRequest {
                    run_id,
                    step_id,
                    llm_response: payload,
                };

                let (_, step_path) = FakeHttpClient::split_url(&step_url);
                let step_resp_json = client.dispatch_evals_post(step_path, &step_req).await;
                let step_resp: StepResponse = serde_json::from_value(step_resp_json)
                    .map_err(|e| format!("step decode: {e}"))?;
                run_id = step_resp.run_id;
                next = step_resp.next;
            }
        }
    }

    Err(format!(
        "eval loop exceeded {} steps without Done",
        MAX_STEPS_PER_RUN
    ))
}

// =============================================================================
// Section 5. Test fixtures.
// =============================================================================

fn fixture_summary() -> SummaryRow {
    // Multi-section body with enough sections to produce 14 metrics on the
    // first run — matches the fixture at `cache_hit.rs:364-378`.
    let body: String = "## Overview\nThe hippocampus is a brain region.\n\n\
                        ## Anatomy & Connectivity\nIt sits in the medial temporal lobe.\n\n\
                        ## Functions\nIt supports declarative memory.\n\n\
                        ## Associated Disorders\nIt is implicated in Alzheimer's.\n\n\
                        ## Symptoms of Damage or Dysfunction\nMemory loss is typical.\n\n\
                        ## Research Highlights\nMany papers exist.\n"
        .repeat(8);
    SummaryRow {
        id: Uuid::new_v4(),
        region_id: 1,
        name: "Hippocampus".to_string(),
        acronym: Some("HIP".to_string()),
        summary: body,
    }
}

fn make_app(db: Arc<InMemoryDb>) -> Arc<EvalsApp<InMemoryDb, DummyEnv, MockError>> {
    let cfg = EvalRuntimeConfig {
        database_url: "memory://".to_string(),
        eval_version: "v0.2.0".to_string(),
        judge_chat_model: "mock-judge".to_string(),
        rubric_chat_model: "mock-rubric".to_string(),
        embedding_model: "mock-embed".to_string(),
        top_k_chunks: 3,
        similarity_threshold: 0.5,
        citation_support_enabled: false,
        citation_support_max_calls: 30,
    };
    Arc::new(EvalsApp {
        db,
        env: Arc::new(DummyEnv),
        config: cfg,
    })
}

fn setup() -> (FakeHttpClient, Uuid) {
    let summary = fixture_summary();
    let summary_id = summary.id;
    let db = Arc::new(InMemoryDb::new(summary, vec![summary_id]));
    let app = make_app(db);
    let api = Arc::new(Evals::new(app));
    let router = build_router(ServerState { api });
    (FakeHttpClient::new(router), summary_id)
}

// =============================================================================
// Section 6. THE contract test — full init -> step* -> Done flow.
// =============================================================================

const EVAL_VERSION: &str = "v0.2.0";
const EVALS_BASE: &str = "http://evals-be.internal";
const BRAIN_BASE: &str = "http://brainatlas.internal";

#[tokio::test]
async fn full_init_step_done_round_trip_through_http() {
    let (client, summary_id) = setup();

    let (step_count, metrics) = drive_one(
        &client,
        EVALS_BASE,
        BRAIN_BASE,
        summary_id,
        EVAL_VERSION,
    )
    .await
    .expect("eval loop should complete without error");

    // The state machine must run the full pipeline: structural (4) +
    // groundedness (2) + rubric (5) + gated rubric (5) + deterministic
    // citation (3) = 19. Matches `cache_hit.rs:454-459`.
    assert_eq!(
        metrics.len(),
        19,
        "first run must produce 19 metrics, got {}",
        metrics.len()
    );

    // And there must have been at least one CallLlm step — otherwise the
    // loop short-circuited to Done without exercising the wire protocol
    // (e.g. everything was already cached, or the router returned Done
    // prematurely).
    assert!(
        step_count > 0,
        "must have issued at least one CallLlm step, got {step_count}"
    );

    // Every metric must have a score in the expected [0.0, 1.0] range.
    for m in &metrics {
        assert!(
            (0.0..=1.0).contains(&m.score),
            "metric {} score {} out of range",
            m.metric,
            m.score
        );
    }
}

// =============================================================================
// Section 7. Cache-hit re-run — asserts the init shortcut path on the wire.
//            After a first full run the scores are persisted; the second
//            init_score request should immediately return Done with all
//            metrics marked cached.
// =============================================================================

#[tokio::test]
async fn second_run_hits_cache_and_returns_done_without_any_step() {
    let (client, summary_id) = setup();

    // First run — populates the cache.
    let (_, first_metrics) = drive_one(
        &client,
        EVALS_BASE,
        BRAIN_BASE,
        summary_id,
        EVAL_VERSION,
    )
    .await
    .expect("first run");
    assert_eq!(first_metrics.len(), 19);

    // Second run — must short-circuit to Done on init.
    let (step_count, metrics) = drive_one(
        &client,
        EVALS_BASE,
        BRAIN_BASE,
        summary_id,
        EVAL_VERSION,
    )
    .await
    .expect("second run");

    assert_eq!(
        step_count, 0,
        "second run must issue ZERO CallLlm steps (full cache hit)"
    );
    assert_eq!(metrics.len(), 19);
    assert!(
        metrics.iter().all(|m| m.cached),
        "every metric on the second run must be cached=true"
    );
}

// =============================================================================
// Section 8. Protocol-path contract — asserts the exact HTTP paths used by
//            orch match the routes mounted by evals-be. Flakiness here is
//            the canary signal the wire protocol has drifted.
// =============================================================================

#[tokio::test]
async fn orch_uses_the_exact_evals_be_nested_paths() {
    let (client, summary_id) = setup();
    let _ = drive_one(
        &client,
        EVALS_BASE,
        BRAIN_BASE,
        summary_id,
        EVAL_VERSION,
    )
    .await
    .expect("drive ok");

    let calls = client.calls().all();
    // Exactly one init call to the nested path.
    let init_calls: Vec<&(String, String, Option<Bytes>)> = calls
        .iter()
        .filter(|(m, u, _)| m == "POST" && u == "/evals-be/api/evals/score/init")
        .collect();
    assert_eq!(
        init_calls.len(),
        1,
        "expected exactly one POST to init path, saw: {:?}",
        calls.iter().map(|c| &c.1).collect::<Vec<_>>()
    );

    // At least one step call, all to the nested step path.
    let step_calls: Vec<&(String, String, Option<Bytes>)> = calls
        .iter()
        .filter(|(m, u, _)| m == "POST" && u == "/evals-be/api/evals/score/step")
        .collect();
    assert!(
        !step_calls.is_empty(),
        "expected at least one POST to step path"
    );
}

// =============================================================================
// Section 9. Correlation-id propagation — orch's contract is to inject
//            `correlation_id = "eval:{run_id}:{step_id}"` into every
//            brainatlas body. Asserting this on the fake brainatlas side
//            guards against orch forgetting to tag its LLM calls (which
//            would break the cost-aggregation feature).
// =============================================================================

#[tokio::test]
async fn orch_injects_correlation_id_on_every_llm_call() {
    let (client, summary_id) = setup();
    let _ = drive_one(
        &client,
        EVALS_BASE,
        BRAIN_BASE,
        summary_id,
        EVAL_VERSION,
    )
    .await
    .expect("drive ok");

    let calls = client.calls().all();
    let llm_calls: Vec<&(String, String, Option<Bytes>)> = calls
        .iter()
        .filter(|(m, u, _)| m == "POST" && u.starts_with(BRAIN_BASE))
        .collect();
    assert!(
        !llm_calls.is_empty(),
        "expected at least one brainatlas call"
    );
    for (_, url, body) in llm_calls {
        let body = body.as_ref().expect("brainatlas call must have body");
        let v: serde_json::Value =
            serde_json::from_slice(body).expect("brainatlas body is JSON");
        let corr = v
            .get("correlation_id")
            .and_then(|x| x.as_str())
            .unwrap_or_else(|| panic!("correlation_id missing on {url}: {v}"));
        assert!(
            corr.starts_with("eval:"),
            "correlation_id must start with 'eval:', saw {corr}"
        );
        // Shape: "eval:{run_id-uuid}:{step_id-uuid}"
        let parts: Vec<&str> = corr.split(':').collect();
        assert_eq!(parts.len(), 3, "correlation_id shape: eval:run:step");
        Uuid::parse_str(parts[1]).expect("run_id component is a UUID");
        Uuid::parse_str(parts[2]).expect("step_id component is a UUID");
    }
}

// =============================================================================
// Section 10. Unknown summary path — evals-be responds 404, orch-mirror
//             surfaces it as a loop failure. Guards the error path of the
//             wire protocol, not just the happy path.
// =============================================================================

#[tokio::test]
async fn unknown_summary_id_returns_404_through_the_router() {
    // Build a router with an empty DB — no summary registered.
    let db = Arc::new(InMemoryDb::default());
    let app = make_app(db);
    let api = Arc::new(Evals::new(app));
    let router = build_router(ServerState { api });

    let missing_summary_id = Uuid::new_v4();
    let req = Request::builder()
        .method("POST")
        .uri("/evals-be/api/evals/score/init")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&InitScoreRequest {
                summary_id: missing_summary_id,
                eval_version: Some(EVAL_VERSION.to_string()),
            })
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(v.get("error").is_some(), "404 body must have 'error' field");
}
