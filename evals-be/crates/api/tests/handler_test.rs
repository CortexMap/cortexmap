//! Axum handler tests for evals-be's HTTP surface.
//!
//! Exercises the real `Router` (built via `api::build_router`) with an
//! in-memory `EvalsDatabase` fake. No Postgres, no network. Uses
//! `tower::ServiceExt::oneshot` to drive request/response end-to-end.
//!
//! Fake pattern mirrors `evals-be/crates/app/tests/cache_hit.rs:38-206`.

use std::sync::Arc;
use std::sync::Mutex;

use app::{EvalRuntimeConfig, EvalsApp};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::NaiveDateTime;
use domain::{EvalRun, EvalRunStatus, EvalScore, NewEvalScore};
use evals_api::{Evals, build_router};
use http_body_util::BodyExt;
use rpc_types::{LlmEndpoint, NextAction};
use services::{
    ChunkRow, EnvInfra, EvalAggregate, EvalsDatabase, MetricStatsRaw, RetrievedChunk, SummaryRow,
    WorstOffenderRow,
};
use tower::ServiceExt;
use uuid::Uuid;

// ---- Mock infra error ----

#[derive(Debug, thiserror::Error)]
#[error("mock infra error: {0}")]
struct MockError(String);

// ---- In-memory EvalsDatabase (copy of the gold-standard in `cache_hit.rs`) ----

#[derive(Default)]
struct InMemoryDb {
    summaries: Mutex<Vec<SummaryRow>>,
    scores: Mutex<Vec<EvalScore>>,
    runs: Mutex<Vec<EvalRun>>,
    run_states: Mutex<Vec<RunStateRow>>,
    worst_offenders: Mutex<Vec<WorstOffenderRow>>,
    aggregate: Mutex<Option<EvalAggregate>>,
    unscored_ids: Mutex<Vec<Uuid>>,
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
    fn with_summary(summary: SummaryRow) -> Self {
        Self {
            summaries: Mutex::new(vec![summary]),
            ..Default::default()
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
        let s = self.summaries.lock().unwrap();
        Ok(s.iter().find(|r| r.id == summary_id).cloned())
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
        Ok(self.aggregate.lock().unwrap().clone().unwrap_or_default())
    }

    async fn get_worst_offenders(
        &self,
        _database_url: &str,
        metric: &str,
        _eval_version: &str,
        limit: i64,
    ) -> Result<Vec<WorstOffenderRow>, Self::Error> {
        Ok(self
            .worst_offenders
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.metric == metric)
            .take(limit.max(0) as usize)
            .cloned()
            .collect())
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
        limit: i64,
    ) -> Result<Vec<Uuid>, Self::Error> {
        Ok(self
            .unscored_ids
            .lock()
            .unwrap()
            .iter()
            .take(limit.max(0) as usize)
            .copied()
            .collect())
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

    async fn delete_run_state(&self, _database_url: &str, run_id: Uuid) -> Result<(), Self::Error> {
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

// ---- Dummy env ----

struct DummyEnv;

impl EnvInfra for DummyEnv {
    type Error = MockError;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
        Err(MockError(format!("no env var {}", key)))
    }
}

// ---- Fixture / router builder ----

fn fixture_summary() -> SummaryRow {
    // Use a body large enough that structural metrics have something to chew on.
    let body: String = "## Overview\nThe hippocampus is a brain region.\n\n\
                        ## Anatomy & Connectivity\nIt sits in the medial temporal lobe.\n\n\
                        ## Functions\nIt supports declarative memory.\n\n\
                        ## Associated Disorders\nIt is implicated in Alzheimer's.\n\n\
                        ## Symptoms of Damage or Dysfunction\nMemory loss is typical.\n\n\
                        ## Research Highlights\nMany papers exist.\n"
        .repeat(4);
    SummaryRow {
        id: Uuid::new_v4(),
        region_id: 1,
        name: "Hippocampus".to_string(),
        acronym: Some("HIP".to_string()),
        summary: body,
    }
}

fn runtime_config() -> EvalRuntimeConfig {
    EvalRuntimeConfig {
        database_url: "memory://".to_string(),
        eval_version: "v0.2.0".to_string(),
        judge_chat_model: "mock-judge".to_string(),
        rubric_chat_model: "mock-rubric".to_string(),
        embedding_model: "mock-embed".to_string(),
        top_k_chunks: 3,
        similarity_threshold: 0.5,
        citation_support_enabled: false,
        citation_support_max_calls: 30,
    }
}

fn build_test_router(db: Arc<InMemoryDb>) -> Router {
    let app = EvalsApp {
        db,
        env: Arc::new(DummyEnv),
        config: runtime_config(),
    };
    let api = Arc::new(Evals::new(Arc::new(app)));
    build_router(api)
}

// ---- Helpers ----

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    resp.into_body()
        .collect()
        .await
        .expect("body collect")
        .to_bytes()
        .to_vec()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = body_bytes(resp).await;
    serde_json::from_slice(&bytes).expect("response body is valid JSON")
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ---- Tests ----

#[tokio::test]
async fn health_returns_200_ok() {
    let db = Arc::new(InMemoryDb::default());
    let router = build_test_router(db);

    let resp = router.oneshot(get("/evals-be/health")).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn init_score_happy_path_returns_run_id_and_call_llm() {
    let summary = fixture_summary();
    let summary_id = summary.id;
    let db = Arc::new(InMemoryDb::with_summary(summary));
    let router = build_test_router(db.clone());

    let req = post_json(
        "/evals-be/api/evals/score/init",
        serde_json::json!({"summary_id": summary_id}),
    );
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["summary_id"], summary_id.to_string());
    assert_eq!(json["eval_version"], "v0.2.0");
    assert!(
        json["run_id"].is_string(),
        "run_id must be a string uuid, got {:?}",
        json["run_id"]
    );
    // First real action after structural metrics must be a CallLlm
    // (ExtractClaims). We only assert the discriminant so the test is not
    // coupled to the ordering of every embedded field.
    assert_eq!(
        json["next"]["kind"], "call_llm",
        "expected next.kind=call_llm, got {}",
        json["next"]
    );
    assert_eq!(json["next"]["endpoint"], "extract_claims");

    // run_state row persisted in the fake DB.
    assert_eq!(
        db.run_states.lock().unwrap().len(),
        1,
        "exactly one run_state row must be persisted"
    );
}

#[tokio::test]
async fn init_score_unknown_summary_returns_404() {
    let db = Arc::new(InMemoryDb::default());
    let router = build_test_router(db);

    let req = post_json(
        "/evals-be/api/evals/score/init",
        serde_json::json!({"summary_id": Uuid::new_v4()}),
    );
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "summary not found");
}

#[tokio::test]
async fn init_score_missing_required_field_returns_400() {
    // InitScoreRequest requires `summary_id`. axum's Json extractor returns
    // 422 by default for malformed JSON, but for missing required fields
    // axum 0.8 returns 422 Unprocessable Entity. We assert it is a 4xx in
    // the "client error" range and that the handler was never reached (no
    // run_state row).
    let db = Arc::new(InMemoryDb::default());
    let router = build_test_router(db.clone());

    let req = post_json(
        "/evals-be/api/evals/score/init",
        serde_json::json!({"eval_version": "v0.2.0"}), // missing summary_id
    );
    let resp = router.oneshot(req).await.unwrap();

    assert!(
        resp.status().is_client_error(),
        "expected 4xx for missing required field, got {}",
        resp.status()
    );
    assert_eq!(
        db.run_states.lock().unwrap().len(),
        0,
        "handler must not have executed"
    );
}

#[tokio::test]
async fn init_score_idempotent_re_init_replaces_prior_run_state() {
    // The app's contract is: re-initing the same (summary_id, eval_version)
    // deletes any abandoned run_state row before inserting a new one, so the
    // caller always gets a fresh run_id and exactly one row remains in-flight.
    let summary = fixture_summary();
    let summary_id = summary.id;
    let db = Arc::new(InMemoryDb::with_summary(summary));
    let router = build_test_router(db.clone());

    let req1 = post_json(
        "/evals-be/api/evals/score/init",
        serde_json::json!({"summary_id": summary_id}),
    );
    let resp1 = router.clone().oneshot(req1).await.unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    let json1 = body_json(resp1).await;
    let run_id_1 = json1["run_id"].as_str().unwrap().to_string();

    let req2 = post_json(
        "/evals-be/api/evals/score/init",
        serde_json::json!({"summary_id": summary_id}),
    );
    let resp2 = router.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let json2 = body_json(resp2).await;
    let run_id_2 = json2["run_id"].as_str().unwrap().to_string();

    assert_ne!(
        run_id_1, run_id_2,
        "each init must mint a fresh run_id (abandoned run cleaned up)"
    );
    // Exactly one in-flight run remains (the second one).
    let rs = db.run_states.lock().unwrap();
    assert_eq!(rs.len(), 1, "abandoned first run_state must be deleted");
    assert_eq!(rs[0].run_id.to_string(), run_id_2);
}

#[tokio::test]
async fn step_score_unknown_run_id_returns_400() {
    let db = Arc::new(InMemoryDb::default());
    let router = build_test_router(db);

    // Valid InitScoreRequest shape on the wire but the run_id has never been
    // inserted → app.step_score returns `AppError::InvalidArg` → 400.
    let req = post_json(
        "/evals-be/api/evals/score/step",
        serde_json::json!({
            "run_id": Uuid::new_v4(),
            "step_id": Uuid::new_v4(),
            "llm_response": {"kind": "embed", "embedding": [0.1, 0.2, 0.3]}
        }),
    );
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    let err = json["error"].as_str().unwrap();
    assert!(
        err.contains("unknown run_id"),
        "expected 'unknown run_id' error, got {}",
        err
    );
}

#[tokio::test]
async fn step_score_missing_required_field_returns_4xx() {
    let db = Arc::new(InMemoryDb::default());
    let router = build_test_router(db);

    // No `llm_response` at all.
    let req = post_json(
        "/evals-be/api/evals/score/step",
        serde_json::json!({"run_id": Uuid::new_v4(), "step_id": Uuid::new_v4()}),
    );
    let resp = router.oneshot(req).await.unwrap();

    assert!(
        resp.status().is_client_error(),
        "expected 4xx, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn step_score_drives_state_machine_to_next_action() {
    // Drive one real step: init → first CallLlm → POST step with the fake
    // embed payload → another CallLlm (ExtractClaims has already run; next
    // is JudgeGroundedness). This exercises the full path through
    // `app.step_score` including `save_run_state`.
    let summary = fixture_summary();
    let summary_id = summary.id;
    let db = Arc::new(InMemoryDb::with_summary(summary));
    let router = build_test_router(db.clone());

    let init_resp = router
        .clone()
        .oneshot(post_json(
            "/evals-be/api/evals/score/init",
            serde_json::json!({"summary_id": summary_id}),
        ))
        .await
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);
    let init_json = body_json(init_resp).await;
    let run_id: Uuid = init_json["run_id"].as_str().unwrap().parse().unwrap();
    let next = &init_json["next"];
    assert_eq!(next["kind"], "call_llm");
    assert_eq!(next["endpoint"], "extract_claims");
    let step_id: Uuid = next["step_id"].as_str().unwrap().parse().unwrap();

    // Feed a Claims response so the state machine advances.
    let step_resp = router
        .oneshot(post_json(
            "/evals-be/api/evals/score/step",
            serde_json::json!({
                "run_id": run_id,
                "step_id": step_id,
                "llm_response": {
                    "kind": "claims",
                    "claims": [
                        {"id": 1, "section": "Overview", "text": "The hippocampus supports memory.", "cited_chunks": []}
                    ]
                }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(step_resp.status(), StatusCode::OK);
    let step_json = body_json(step_resp).await;
    assert_eq!(step_json["run_id"], run_id.to_string());
    // The next action can be either another CallLlm (embed) or Done. The
    // state machine chooses; we only require the discriminant be valid and
    // that the run_state row for this run_id is still alive (or cleaned up
    // if Done). Either is a correct handler response.
    let kind = step_json["next"]["kind"].as_str().unwrap();
    assert!(
        kind == "call_llm" || kind == "done",
        "unexpected next.kind: {}",
        kind
    );
    // Verify that LlmEndpoint::Embed is a legal variant on the wire so the
    // test fails loudly if rpc-types is silently renamed.
    assert_eq!(LlmEndpoint::Embed.path(), "/brainatlas-be/api/llm/embed");
    let _ = NextAction::Done {
        metrics: Vec::new(),
    };
}

#[tokio::test]
async fn scores_for_summary_returns_seeded_scores() {
    let summary = fixture_summary();
    let summary_id = summary.id;
    let db = Arc::new(InMemoryDb::with_summary(summary));
    // Pre-seed one score row.
    db.scores.lock().unwrap().push(EvalScore {
        id: Uuid::new_v4(),
        summary_id,
        summary_hash: "hash".to_string(),
        metric: "rubric_relevance".to_string(),
        score: 0.75,
        judge_model: Some("gpt-4o-mini".to_string()),
        details: None,
        eval_version: "v0.2.0".to_string(),
        created_at: NaiveDateTime::default(),
    });

    let router = build_test_router(db);

    let resp = router
        .oneshot(get(&format!("/evals-be/api/evals/scores/{}", summary_id)))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let scores = json["scores"].as_array().unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0]["metric"], "rubric_relevance");
    assert!((scores[0]["score"].as_f64().unwrap() - 0.75).abs() < 1e-6);
}

#[tokio::test]
async fn scores_for_summary_invalid_uuid_returns_400() {
    let db = Arc::new(InMemoryDb::default());
    let router = build_test_router(db);

    let resp = router
        .oneshot(get("/evals-be/api/evals/scores/not-a-uuid"))
        .await
        .unwrap();

    // axum's Path extractor rejects a non-UUID with 400 Bad Request.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn aggregate_summary_returns_empty_aggregate() {
    let db = Arc::new(InMemoryDb::default());
    let router = build_test_router(db);

    let resp = router
        .oneshot(get("/evals-be/api/evals/summary"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["eval_version"], "v0.2.0");
    assert_eq!(json["total_summaries"], 0);
    assert_eq!(json["total_scored"], 0);
    assert!(json["per_metric"].is_object());
}

#[tokio::test]
async fn aggregate_summary_honors_eval_version_query() {
    let db = Arc::new(InMemoryDb::default());
    let mut agg = EvalAggregate {
        total_summaries: 7,
        total_scored: 3,
        ..Default::default()
    };
    agg.per_metric.insert(
        "rubric_relevance".to_string(),
        MetricStatsRaw {
            avg: 0.8,
            min: 0.1,
            max: 1.0,
            count: 3,
        },
    );
    *db.aggregate.lock().unwrap() = Some(agg);

    let router = build_test_router(db);

    let resp = router
        .oneshot(get("/evals-be/api/evals/summary?eval_version=v9"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["eval_version"], "v9");
    assert_eq!(json["total_summaries"], 7);
    assert_eq!(json["total_scored"], 3);
    assert!(
        (json["per_metric"]["rubric_relevance"]["avg"]
            .as_f64()
            .unwrap()
            - 0.8)
            .abs()
            < 1e-6
    );
}

#[tokio::test]
async fn worst_offenders_happy_path() {
    let db = Arc::new(InMemoryDb::default());
    db.worst_offenders.lock().unwrap().push(WorstOffenderRow {
        summary_id: Uuid::new_v4(),
        region_name: Some("Amygdala".to_string()),
        metric: "rubric_relevance".to_string(),
        score: 0.05,
        eval_version: "v0.2.0".to_string(),
    });
    let router = build_test_router(db);

    let resp = router
        .oneshot(get(
            "/evals-be/api/evals/worst?metric=rubric_relevance&limit=5",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["metric"], "rubric_relevance");
    assert_eq!(json["limit"], 5);
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["region_name"], "Amygdala");
}

#[tokio::test]
async fn worst_offenders_missing_metric_returns_4xx() {
    let db = Arc::new(InMemoryDb::default());
    let router = build_test_router(db);

    let resp = router
        .oneshot(get("/evals-be/api/evals/worst"))
        .await
        .unwrap();

    // `metric` is required; axum's Query extractor rejects the missing field.
    assert!(
        resp.status().is_client_error(),
        "expected 4xx for missing `metric`, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn unscored_endpoint_returns_ids() {
    let id_a = Uuid::new_v4();
    let id_b = Uuid::new_v4();
    let db = Arc::new(InMemoryDb::default());
    db.unscored_ids
        .lock()
        .unwrap()
        .extend_from_slice(&[id_a, id_b]);
    let router = build_test_router(db);

    let resp = router
        .oneshot(get("/evals-be/api/evals/unscored?limit=10"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["limit"], 10);
    assert_eq!(json["eval_version"], "v0.2.0");
    let ids = json["summary_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
}
