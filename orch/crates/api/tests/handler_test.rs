//! Axum handler integration tests for orch.
//!
//! Exercises the real `Router` built by `server::OrchServer::into_router()` via
//! `tower::ServiceExt::oneshot` against an in-memory `Services` fake.
//!
//! Covered PR #69 routes:
//!   * POST /orch/api/pipeline/trigger  (manual pipeline trigger + per-phase opt-in)
//!   * GET  /orch/dev/api/redis-stats   (redis stats)
//!   * GET  /orch/dev/api/system-stats  (dev dashboard)
//!   * GET  /orch/dev/api/summary-freshness (dev dashboard)
//!   * GET  /orch/dev/stats             (static dashboard HTML)
//!   * GET  /orch/api/evals/status      (eval status passthrough)
//!   * GET  /orch/api/evals/worst       (eval worst offenders with query params)
//!   * GET  /orch/api/evals/runs/{id}/cost (eval run cost)
//!
//! Plus 400 / 404 / 500 / body-validation paths.

use api::Orch;
use app::{
    BatchOrchestration, CompletionOrchestrator, ConfigManagement, CostGuardrailOrchestration,
    EvalOrchestration, EvalStatusSummary, EvalWorstOffenders, HealthCheck, PipelineRunner,
    RegionManagement, WorkerManagement,
};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use domain::{
    AllocateWorkersRequest, BatchStatus, ConfigEntry, ConfigEntryUpdate, ConfigKey, PendingTask,
    PollResult, ProcessResult, ProcessingBatch, RedisPrefixCount, RedisStats, Region, RegionQuery,
    RegionSummary, StopWorkersRequest, SummaryFreshness, SystemStats, WorkerAllocationResponse,
    WorkerStatus, WorkerStopResponse,
};
use http_body_util::BodyExt;
use server::OrchServer;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Fake error + fake Services
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct FakeErr(String);

impl std::fmt::Display for FakeErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FakeErr {}

/// In-memory `Services` fake. Each method returns the corresponding "canned"
/// `Result` from the relevant `Mutex<...>` slot. Unused methods return an
/// `Err(FakeErr("not staged"))` so an unexpected call surfaces as a 500 in the
/// test (never a panic that would poison tokio).
#[derive(Default)]
struct FakeServices {
    // Canned outcomes for routes we explicitly exercise.
    redis_stats: Mutex<Option<Result<RedisStats, FakeErr>>>,
    system_stats: Mutex<Option<Result<SystemStats, FakeErr>>>,
    summary_freshness: Mutex<Option<Result<SummaryFreshness, FakeErr>>>,
    pipeline_trigger_delete_count: Mutex<Option<i64>>,
    pipeline_trigger_gen_queries: Mutex<Option<(usize, usize)>>,
    pipeline_trigger_discover: Mutex<Option<(usize, usize)>>,
    eval_status: Mutex<Option<Result<EvalStatusSummary, FakeErr>>>,
    eval_worst: Mutex<Option<Result<EvalWorstOffenders, FakeErr>>>,
    eval_run_cost: Mutex<Option<Result<domain::EvalRunCost, FakeErr>>>,
    get_config: Mutex<Option<Result<Vec<ConfigEntry>, FakeErr>>>,
    update_config_result: Mutex<Option<Result<Vec<ConfigEntry>, FakeErr>>>,
    list_summaries: Mutex<Option<Result<Vec<RegionSummary>, FakeErr>>>,
    active_batch: Mutex<Option<Result<Option<ProcessingBatch>, FakeErr>>>,
    recent_batch: Mutex<Option<Result<Option<ProcessingBatch>, FakeErr>>>,
    batch_by_id: Mutex<Option<Result<Option<ProcessingBatch>, FakeErr>>>,
    count_completed_tasks: Mutex<Option<Result<i32, FakeErr>>>,
    all_regions: Mutex<Option<Result<Vec<Region>, FakeErr>>>,
    worker_status: Mutex<Option<Result<Vec<WorkerStatus>, FakeErr>>>,
    allocate_workers_result: Mutex<Option<Result<WorkerAllocationResponse, FakeErr>>>,
    stop_workers_result: Mutex<Option<Result<WorkerStopResponse, FakeErr>>>,
    chunk_source: Mutex<Option<Result<domain::ChunkSourceResponse, FakeErr>>>,
    reverse_search_result: Mutex<Option<Result<domain::SearchResponse, FakeErr>>>,
    total_regions: Mutex<Option<Result<i64, FakeErr>>>,
    regions_without_batches: Mutex<Option<Result<i64, FakeErr>>>,
    actively_fetching_regions: Mutex<Option<Result<i64, FakeErr>>>,
    batches_by_status: Mutex<Option<Vec<(BatchStatus, Vec<ProcessingBatch>)>>>,
    pending_fetch_task_count: Mutex<Option<Result<i64, FakeErr>>>,
    regions_without_queries_count: Mutex<Option<Result<i64, FakeErr>>>,
    regions_with_queries_count: Mutex<Option<Result<i64, FakeErr>>>,
    // Last args observed by a handful of setter-style handlers.
    last_worst_args: Mutex<Option<(String, i64)>>,
    last_update_config: Mutex<Option<Vec<ConfigEntryUpdate>>>,
    last_allocate_req: Mutex<Option<AllocateWorkersRequest>>,
    last_stop_req: Mutex<Option<StopWorkersRequest>>,
    last_reverse_query: Mutex<Option<String>>,
}

impl FakeServices {
    fn new() -> Self {
        Self::default()
    }
}

fn not_staged<T>(name: &'static str) -> Result<T, FakeErr> {
    Err(FakeErr(format!("not staged: {name}")))
}

// ---- CompletionOrchestrator ----

#[async_trait::async_trait]
impl CompletionOrchestrator for FakeServices {
    type Error = FakeErr;

    async fn poll(&self) -> Result<PollResult, Self::Error> {
        not_staged("poll")
    }
    async fn process(&self, _tasks: Vec<PendingTask>) -> Result<ProcessResult, Self::Error> {
        not_staged("process")
    }
    async fn get_config(&self, _key: ConfigKey) -> Result<Option<String>, Self::Error> {
        Ok(None)
    }
}

// ---- RegionManagement ----

#[async_trait::async_trait]
impl RegionManagement for FakeServices {
    type Error = FakeErr;

    async fn get_summaries(&self, _region_id: Uuid) -> Result<Vec<RegionSummary>, Self::Error> {
        match self.list_summaries.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_summaries"),
        }
    }
    async fn get_active_batch(
        &self,
        _region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        match self.active_batch.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_active_batch"),
        }
    }
    async fn get_recent_batch(
        &self,
        _region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        match self.recent_batch.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_recent_batch"),
        }
    }
    async fn get_queries(&self, _region_id: Uuid) -> Result<Vec<RegionQuery>, Self::Error> {
        not_staged("get_queries")
    }
    async fn store_queries(
        &self,
        _region_id: Uuid,
        _queries: Vec<String>,
    ) -> Result<Vec<Uuid>, Self::Error> {
        not_staged("store_queries")
    }
    async fn generate_queries(
        &self,
        _region_name: &str,
        _count: u32,
    ) -> Result<Vec<String>, Self::Error> {
        not_staged("generate_queries")
    }
    async fn update_batch_status(
        &self,
        _batch_id: Uuid,
        _status: BatchStatus,
        _error: Option<String>,
    ) -> Result<(), Self::Error> {
        not_staged("update_batch_status")
    }
    async fn get_batches_by_status(
        &self,
        status: BatchStatus,
    ) -> Result<Vec<ProcessingBatch>, Self::Error> {
        if let Some(list) = self.batches_by_status.lock().unwrap().as_ref() {
            for (s, v) in list {
                if *s == status {
                    return Ok(v.clone());
                }
            }
            return Ok(vec![]);
        }
        not_staged("get_batches_by_status")
    }
    async fn get_region_name(&self, _region_id: Uuid) -> Result<String, Self::Error> {
        not_staged("get_region_name")
    }
    async fn get_total_regions(&self) -> Result<i64, Self::Error> {
        match self.total_regions.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_total_regions"),
        }
    }
    async fn count_regions_without_batches(&self) -> Result<i64, Self::Error> {
        match self.regions_without_batches.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("count_regions_without_batches"),
        }
    }
    async fn count_actively_fetching_regions(&self) -> Result<i64, Self::Error> {
        match self.actively_fetching_regions.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("count_actively_fetching_regions"),
        }
    }
    async fn get_latest_active_summary_age(
        &self,
        _region_id: Uuid,
    ) -> Result<Option<chrono::NaiveDateTime>, Self::Error> {
        not_staged("get_latest_active_summary_age")
    }
    async fn get_summary_freshness(&self) -> Result<SummaryFreshness, Self::Error> {
        match self.summary_freshness.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_summary_freshness"),
        }
    }
    async fn get_query_generation_limit(&self) -> Result<Option<u32>, Self::Error> {
        not_staged("get_query_generation_limit")
    }
    async fn get_all_regions(&self) -> Result<Vec<Region>, Self::Error> {
        match self.all_regions.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_all_regions"),
        }
    }
    async fn delete_queries(&self, _region_id: Uuid) -> Result<(), Self::Error> {
        not_staged("delete_queries")
    }
    async fn delete_all_queries(&self) -> Result<i64, Self::Error> {
        // Used by trigger_pipeline's reset phase.
        match *self.pipeline_trigger_delete_count.lock().unwrap() {
            Some(n) => Ok(n),
            None => not_staged("delete_all_queries"),
        }
    }
    async fn get_chunk_source(
        &self,
        _chunk_id: Uuid,
    ) -> Result<domain::ChunkSourceResponse, Self::Error> {
        match self.chunk_source.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_chunk_source"),
        }
    }
    async fn reverse_search(&self, query: &str) -> Result<domain::SearchResponse, Self::Error> {
        *self.last_reverse_query.lock().unwrap() = Some(query.to_string());
        match self.reverse_search_result.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("reverse_search"),
        }
    }
}

// ---- BatchOrchestration ----

#[async_trait::async_trait]
impl BatchOrchestration for FakeServices {
    type Error = FakeErr;

    async fn create_batch(
        &self,
        _region_id: Uuid,
        _expected_count: usize,
    ) -> Result<Uuid, Self::Error> {
        not_staged("create_batch")
    }
    async fn enqueue_fetch_task(
        &self,
        _query: String,
        _region_id: Uuid,
        _priority: i32,
    ) -> Result<Vec<i64>, Self::Error> {
        not_staged("enqueue_fetch_task")
    }
    async fn add_tasks_to_batch(
        &self,
        _batch_id: Uuid,
        _task_ids: Vec<i64>,
    ) -> Result<(), Self::Error> {
        not_staged("add_tasks_to_batch")
    }
    async fn update_batch_expected_count(
        &self,
        _batch_id: Uuid,
        _count: i32,
    ) -> Result<(), Self::Error> {
        not_staged("update_batch_expected_count")
    }
    async fn get_batch_by_id(
        &self,
        _batch_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        match self.batch_by_id.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_batch_by_id"),
        }
    }
    async fn ensure_workers_allocated(&self) -> Result<(), Self::Error> {
        // Used by trigger_pipeline's ensure_workers phase. Succeed silently.
        Ok(())
    }
    async fn count_completed_tasks(&self, _task_ids: Vec<i64>) -> Result<i32, Self::Error> {
        match self.count_completed_tasks.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("count_completed_tasks"),
        }
    }
    async fn get_completed_task_ids(&self, _task_ids: Vec<i64>) -> Result<Vec<i64>, Self::Error> {
        not_staged("get_completed_task_ids")
    }
}

// ---- ConfigManagement ----

#[async_trait::async_trait]
impl ConfigManagement for FakeServices {
    type Error = FakeErr;

    async fn get_all_config(&self) -> Result<Vec<ConfigEntry>, Self::Error> {
        match self.get_config.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_all_config"),
        }
    }
    async fn update_config(
        &self,
        entries: Vec<ConfigEntryUpdate>,
    ) -> Result<Vec<ConfigEntry>, Self::Error> {
        *self.last_update_config.lock().unwrap() = Some(entries);
        match self.update_config_result.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("update_config"),
        }
    }
}

// ---- WorkerManagement ----

#[async_trait::async_trait]
impl WorkerManagement for FakeServices {
    type Error = FakeErr;

    async fn get_worker_status(&self) -> Result<Vec<WorkerStatus>, Self::Error> {
        match self.worker_status.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_worker_status"),
        }
    }
    async fn allocate_workers(
        &self,
        req: AllocateWorkersRequest,
    ) -> Result<WorkerAllocationResponse, Self::Error> {
        *self.last_allocate_req.lock().unwrap() = Some(req);
        match self.allocate_workers_result.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("allocate_workers"),
        }
    }
    async fn stop_workers(
        &self,
        req: StopWorkersRequest,
    ) -> Result<WorkerStopResponse, Self::Error> {
        *self.last_stop_req.lock().unwrap() = Some(req);
        match self.stop_workers_result.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("stop_workers"),
        }
    }
}

// ---- HealthCheck ----

#[async_trait::async_trait]
impl HealthCheck for FakeServices {
    type Error = FakeErr;

    async fn fetcher_health(&self) -> Result<(), Self::Error> {
        Ok(())
    }
    async fn brainatlas_health(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ---- PipelineRunner ----

#[async_trait::async_trait]
impl PipelineRunner for FakeServices {
    type Error = FakeErr;

    async fn generate_queries_for_new_regions(&self) -> Result<(usize, usize), Self::Error> {
        match *self.pipeline_trigger_gen_queries.lock().unwrap() {
            Some(v) => Ok(v),
            None => not_staged("generate_queries_for_new_regions"),
        }
    }
    async fn discover_new_papers(&self) -> Result<(usize, usize), Self::Error> {
        match *self.pipeline_trigger_discover.lock().unwrap() {
            Some(v) => Ok(v),
            None => not_staged("discover_new_papers"),
        }
    }
    async fn ensure_fetcher_running(&self) -> Result<(), Self::Error> {
        Ok(())
    }
    async fn get_pending_fetch_task_count(&self) -> Result<i64, Self::Error> {
        match self.pending_fetch_task_count.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_pending_fetch_task_count"),
        }
    }
    async fn generate_queries_for_new_regions_count(&self) -> Result<i64, Self::Error> {
        match self.regions_without_queries_count.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("generate_queries_for_new_regions_count"),
        }
    }
    async fn get_regions_with_queries_count(&self) -> Result<i64, Self::Error> {
        match self.regions_with_queries_count.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_regions_with_queries_count"),
        }
    }
    async fn get_system_stats(&self) -> Result<SystemStats, Self::Error> {
        match self.system_stats.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_system_stats"),
        }
    }
    async fn get_redis_stats(&self) -> Result<RedisStats, Self::Error> {
        match self.redis_stats.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("get_redis_stats"),
        }
    }
}

// ---- EvalOrchestration ----

#[async_trait::async_trait]
impl EvalOrchestration for FakeServices {
    type Error = FakeErr;

    async fn eval_orchestrator_enabled(&self) -> bool {
        false
    }
    async fn eval_orchestrator_poll_interval_secs(&self) -> u64 {
        300
    }
    async fn eval_orchestrator_run_cycle(&self) -> Result<(usize, usize), Self::Error> {
        not_staged("eval_orchestrator_run_cycle")
    }
    async fn eval_orchestrator_get_status(&self) -> Result<EvalStatusSummary, Self::Error> {
        match self.eval_status.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("eval_orchestrator_get_status"),
        }
    }
    async fn eval_orchestrator_get_worst(
        &self,
        metric: String,
        limit: i64,
    ) -> Result<EvalWorstOffenders, Self::Error> {
        *self.last_worst_args.lock().unwrap() = Some((metric, limit));
        match self.eval_worst.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("eval_orchestrator_get_worst"),
        }
    }
    async fn eval_orchestrator_get_run_cost(
        &self,
        _run_id: Uuid,
    ) -> Result<domain::EvalRunCost, Self::Error> {
        match self.eval_run_cost.lock().unwrap().take() {
            Some(r) => r,
            None => not_staged("eval_orchestrator_get_run_cost"),
        }
    }
}

// ---- CostGuardrailOrchestration ----

#[async_trait::async_trait]
impl CostGuardrailOrchestration for FakeServices {
    type Error = FakeErr;

    async fn cost_guardrail_enabled(&self) -> bool {
        false
    }
    async fn cost_guardrail_poll_interval_secs(&self) -> u64 {
        300
    }
    async fn cost_guardrail_run_once(&self) -> Option<f64> {
        None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn router(svc: Arc<FakeServices>) -> Router {
    let api = Arc::new(Orch::new(svc));
    let server = OrchServer::new(api);
    server.into_router()
}

async fn read_body_json(resp: axum::http::Response<axum::body::Body>) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).expect("response body is JSON")
}

async fn read_body_bytes(resp: axum::http::Response<axum::body::Body>) -> Vec<u8> {
    resp.into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn sample_redis_stats_connected() -> RedisStats {
    RedisStats {
        connected: true,
        error: None,
        total_keys: 1234,
        keys_by_prefix: vec![RedisPrefixCount {
            pattern: "orch:region:*:status".to_string(),
            description: "region status cache".to_string(),
            count: 42,
        }],
        used_memory_bytes: 1024 * 1024,
        used_memory_human: "1.00M".to_string(),
        uptime_secs: 7200,
        total_connections_received: 11,
        keyspace_hits: 900,
        keyspace_misses: 100,
        hit_rate: 0.9,
        server_version: "7.2.4".to_string(),
    }
}

fn sample_redis_stats_disconnected() -> RedisStats {
    RedisStats {
        connected: false,
        error: Some("PING failed".to_string()),
        total_keys: 0,
        keys_by_prefix: vec![],
        used_memory_bytes: 0,
        used_memory_human: "0B".to_string(),
        uptime_secs: 0,
        total_connections_received: 0,
        keyspace_hits: 0,
        keyspace_misses: 0,
        hit_rate: 0.0,
        server_version: "".to_string(),
    }
}

fn sample_system_stats() -> SystemStats {
    SystemStats {
        fetch_tasks_by_status: vec![domain::StatusCount {
            status: "completed".to_string(),
            count: 500,
        }],
        batches_by_status: vec![],
        total_queries: 200,
        regions_with_queries: 50,
        query_distribution: vec![domain::QueryDistEntry {
            query_count: 3,
            num_regions: 40,
        }],
        total_papers: 1000,
        total_summaries: 120,
        timestamp: chrono::Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_endpoint_returns_200_with_status_ok() {
    let svc = Arc::new(FakeServices::new());
    let app = router(svc);

    let resp = app.oneshot(get("/orch/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let svc = Arc::new(FakeServices::new());
    let app = router(svc);

    let resp = app.oneshot(get("/orch/api/does-not-exist")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_summaries_with_invalid_uuid_path_returns_400() {
    let svc = Arc::new(FakeServices::new());
    let app = router(svc);

    // Axum's Path<Uuid> rejects a malformed UUID with a 400.
    let resp = app
        .oneshot(get("/orch/api/regions/not-a-uuid/summaries"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_config_propagates_service_error_as_500() {
    // Stage a ConfigManagement error and confirm it bubbles out as a 500 with
    // an `{"error": ...}` body (matches the `ServerError::into_response`
    // contract in server.rs).
    let svc = Arc::new(FakeServices::new());
    *svc.get_config.lock().unwrap() = Some(Err(FakeErr("boom: db gone".to_string())));

    let app = router(svc);

    let resp = app.oneshot(get("/orch/api/config")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let body = read_body_json(resp).await;
    assert!(body["error"].is_string());
    assert!(body["error"].as_str().unwrap().contains("boom"));
}

#[tokio::test]
async fn pipeline_trigger_empty_body_is_noop_returning_defaults() {
    // PipelineTriggerRequest defaults every flag to false → no phase runs.
    let svc = Arc::new(FakeServices::new());
    let app = router(svc.clone());

    let resp = app
        .oneshot(post_json(
            "/orch/api/pipeline/trigger",
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    // All per-phase outcomes stay `null` when their flag is false.
    assert!(body["reset_queries_deleted"].is_null());
    assert!(body["generate_queries_result"].is_null());
    assert!(body["discover_papers_result"].is_null());
    assert!(body["ensure_workers_ok"].is_null());
    // `errors` is always present as an array (never omitted).
    assert!(body["errors"].is_array());
    assert_eq!(body["errors"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn pipeline_trigger_with_per_phase_opt_in_runs_selected_phases() {
    let svc = Arc::new(FakeServices::new());
    *svc.pipeline_trigger_delete_count.lock().unwrap() = Some(42);
    *svc.pipeline_trigger_gen_queries.lock().unwrap() = Some((10, 30));
    *svc.pipeline_trigger_discover.lock().unwrap() = Some((5, 12));

    let app = router(svc);

    let req_body = serde_json::json!({
        "reset_queries": true,
        "generate_queries": true,
        "discover_papers": true,
        "ensure_workers": true,
    });

    let resp = app
        .oneshot(post_json("/orch/api/pipeline/trigger", req_body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["reset_queries_deleted"], 42);
    // Tuple serializes as a two-element array.
    assert_eq!(body["generate_queries_result"][0], 10);
    assert_eq!(body["generate_queries_result"][1], 30);
    assert_eq!(body["discover_papers_result"][0], 5);
    assert_eq!(body["discover_papers_result"][1], 12);
    assert_eq!(body["ensure_workers_ok"], true);
    assert!(body["errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn pipeline_trigger_rejects_malformed_json_body_with_400() {
    let svc = Arc::new(FakeServices::new());
    let app = router(svc);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/orch/api/pipeline/trigger")
        .header("content-type", "application/json")
        .body(Body::from("{ this is not json"))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Axum's Json extractor rejects malformed JSON with 400.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn redis_stats_endpoint_returns_connected_payload() {
    let svc = Arc::new(FakeServices::new());
    *svc.redis_stats.lock().unwrap() = Some(Ok(sample_redis_stats_connected()));

    let app = router(svc);

    let resp = app.oneshot(get("/orch/dev/api/redis-stats")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["connected"], true);
    assert_eq!(body["total_keys"], 1234);
    assert_eq!(body["server_version"], "7.2.4");
    assert!((body["hit_rate"].as_f64().unwrap() - 0.9).abs() < 1e-9);
    assert_eq!(body["keys_by_prefix"][0]["pattern"], "orch:region:*:status");
}

#[tokio::test]
async fn redis_stats_endpoint_returns_degraded_payload_when_redis_down() {
    // Contract: a Redis outage surfaces as `connected: false` / `error: Some(..)`
    // at HTTP 200 -- orch intentionally does NOT 500 so the dashboard still
    // renders.
    let svc = Arc::new(FakeServices::new());
    *svc.redis_stats.lock().unwrap() = Some(Ok(sample_redis_stats_disconnected()));

    let app = router(svc);

    let resp = app.oneshot(get("/orch/dev/api/redis-stats")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["connected"], false);
    assert_eq!(body["error"], "PING failed");
    assert_eq!(body["total_keys"], 0);
}

#[tokio::test]
async fn dev_system_stats_endpoint_serializes_all_expected_fields() {
    let svc = Arc::new(FakeServices::new());
    *svc.system_stats.lock().unwrap() = Some(Ok(sample_system_stats()));

    let app = router(svc);

    let resp = app
        .oneshot(get("/orch/dev/api/system-stats"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["total_queries"], 200);
    assert_eq!(body["regions_with_queries"], 50);
    assert_eq!(body["total_papers"], 1000);
    assert_eq!(body["total_summaries"], 120);
    assert_eq!(body["fetch_tasks_by_status"][0]["status"], "completed");
    assert_eq!(body["fetch_tasks_by_status"][0]["count"], 500);
    assert_eq!(body["query_distribution"][0]["query_count"], 3);
    assert_eq!(body["query_distribution"][0]["num_regions"], 40);
    // `timestamp` must be a string (chrono::DateTime<Utc> serializes to RFC3339).
    assert!(body["timestamp"].is_string());
}

#[tokio::test]
async fn dev_summary_freshness_endpoint_returns_expected_shape() {
    let svc = Arc::new(FakeServices::new());
    *svc.summary_freshness.lock().unwrap() = Some(Ok(SummaryFreshness {
        fresh: 800,
        stale: 300,
        no_summary: 100,
        staleness_days: 30,
    }));

    let app = router(svc);

    let resp = app
        .oneshot(get("/orch/dev/api/summary-freshness"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["fresh"], 800);
    assert_eq!(body["stale"], 300);
    assert_eq!(body["no_summary"], 100);
    assert_eq!(body["staleness_days"], 30);
}

#[tokio::test]
async fn dev_stats_page_returns_static_html() {
    let svc = Arc::new(FakeServices::new());
    let app = router(svc);

    let resp = app.oneshot(get("/orch/dev/stats")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("text/html"),
        "expected text/html content-type, got {ct}"
    );

    let bytes = read_body_bytes(resp).await;
    // The static file is the dev_stats.html asset; smoke-check it's non-empty
    // HTML (exact markup intentionally not asserted to keep the test robust
    // against dashboard tweaks).
    assert!(!bytes.is_empty());
    let as_str = String::from_utf8_lossy(&bytes);
    assert!(
        as_str.contains("<html") || as_str.contains("<!DOCTYPE") || as_str.contains("<!doctype"),
        "dev_stats.html should contain an HTML marker; got: {}",
        &as_str[..as_str.len().min(200)]
    );
}

#[tokio::test]
async fn eval_worst_endpoint_applies_default_metric_and_limit() {
    use std::collections::HashMap;
    let svc = Arc::new(FakeServices::new());
    *svc.eval_worst.lock().unwrap() = Some(Ok(EvalWorstOffenders {
        metric: "groundedness".to_string(),
        limit: 10,
        entries: vec![],
    }));

    let app = router(svc.clone());

    // No query params → defaults: metric="groundedness", limit=10.
    let resp = app.oneshot(get("/orch/api/evals/worst")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let _body = read_body_json(resp).await;

    let args = svc.last_worst_args.lock().unwrap().clone();
    assert_eq!(args, Some(("groundedness".to_string(), 10_i64)));

    // Keep `HashMap` import used.
    let _ = HashMap::<String, f32>::new();
}

#[tokio::test]
async fn eval_worst_endpoint_forwards_query_params_to_service() {
    let svc = Arc::new(FakeServices::new());
    *svc.eval_worst.lock().unwrap() = Some(Ok(EvalWorstOffenders {
        metric: "rubric_relevance".to_string(),
        limit: 25,
        entries: vec![],
    }));

    let app = router(svc.clone());

    let resp = app
        .oneshot(get(
            "/orch/api/evals/worst?metric=rubric_relevance&limit=25",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["metric"], "rubric_relevance");
    assert_eq!(body["limit"], 25);

    let args = svc.last_worst_args.lock().unwrap().clone();
    assert_eq!(args, Some(("rubric_relevance".to_string(), 25_i64)));
}

#[tokio::test]
async fn eval_status_endpoint_returns_expected_wire_shape() {
    use std::collections::HashMap;
    let mut per_metric = HashMap::new();
    per_metric.insert(
        "groundedness".to_string(),
        app::EvalMetricStatsView {
            avg: 0.75,
            min: 0.1,
            max: 1.0,
            count: 300,
        },
    );

    let svc = Arc::new(FakeServices::new());
    *svc.eval_status.lock().unwrap() = Some(Ok(EvalStatusSummary {
        eval_version: "v2".to_string(),
        total_summaries: 1000,
        total_scored: 700,
        per_metric,
    }));

    let app = router(svc);

    let resp = app.oneshot(get("/orch/api/evals/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["eval_version"], "v2");
    assert_eq!(body["total_summaries"], 1000);
    assert_eq!(body["total_scored"], 700);
    assert_eq!(body["per_metric"]["groundedness"]["count"], 300);
    assert!((body["per_metric"]["groundedness"]["avg"].as_f64().unwrap() - 0.75).abs() < 1e-6);
}

#[tokio::test]
async fn eval_run_cost_endpoint_returns_string_scalars_for_precision() {
    let svc = Arc::new(FakeServices::new());
    *svc.eval_run_cost.lock().unwrap() = Some(Ok(domain::EvalRunCost {
        run_id: "run-abc".to_string(),
        total_cost_usd: "0.123456789".to_string(),
        total_input_tokens: 5_000,
        total_output_tokens: 2_500,
        total_calls: 7,
    }));

    let app = router(svc);

    let run_id = Uuid::new_v4();
    let resp = app
        .oneshot(get(&format!("/orch/api/evals/runs/{run_id}/cost")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    // `total_cost_usd` is a string on the wire to preserve precision.
    assert_eq!(body["total_cost_usd"], "0.123456789");
    assert_eq!(body["total_input_tokens"], 5000);
    assert_eq!(body["total_output_tokens"], 2500);
    assert_eq!(body["total_calls"], 7);
}

#[tokio::test]
async fn eval_run_cost_endpoint_rejects_invalid_uuid_with_400() {
    let svc = Arc::new(FakeServices::new());
    let app = router(svc);

    let resp = app
        .oneshot(get("/orch/api/evals/runs/not-a-uuid/cost"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pipeline_trigger_preserves_errors_on_partial_phase_failure() {
    // Opt in to all phases but have Phase 1 return staged OK and Phase 2 fail
    // by leaving `pipeline_trigger_discover` None (which yields `not staged`
    // error). The trigger_pipeline orchestrator is expected to SWALLOW the
    // per-phase error and return it in `errors: [..]` rather than 500.
    let svc = Arc::new(FakeServices::new());
    *svc.pipeline_trigger_gen_queries.lock().unwrap() = Some((3, 5));
    // Leave pipeline_trigger_discover unset so discover_new_papers errors.

    let app = router(svc);

    let req_body = serde_json::json!({
        "generate_queries": true,
        "discover_papers": true,
    });
    let resp = app
        .oneshot(post_json("/orch/api/pipeline/trigger", req_body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["generate_queries_result"][0], 3);
    assert_eq!(body["generate_queries_result"][1], 5);
    assert!(body["discover_papers_result"].is_null());
    let errors = body["errors"].as_array().expect("errors array");
    assert!(
        !errors.is_empty(),
        "expected at least one phase error, got {body:#}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.as_str().unwrap_or("").contains("discover")
                || e.as_str().unwrap_or("").contains("Phase 2")
                || e.as_str().unwrap_or("").contains("not staged")),
        "expected the Phase-2 failure surface to include a descriptive message: {body:#}"
    );
}

// ---------------------------------------------------------------------------
// Additional tests: extend coverage of app.rs + server.rs handlers.
// ---------------------------------------------------------------------------

fn patch_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PATCH)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn sample_batch(region_id: Uuid, status: BatchStatus, task_ids: Vec<i64>) -> ProcessingBatch {
    ProcessingBatch {
        id: Uuid::new_v4(),
        region_id,
        status,
        fetch_task_ids: task_ids,
        expected_task_count: 5,
        content_hash: None,
        created_at: chrono::Utc::now(),
        ready_at: None,
        processing_started_at: None,
        completed_at: None,
        summary_id: None,
        error_message: None,
    }
}

fn sample_region_summary(region_id: Uuid) -> RegionSummary {
    let _ = region_id;
    RegionSummary {
        summary_id: Uuid::new_v4(),
        summary: "A brief overview".to_string(),
        created_at: chrono::Utc::now(),
        batch_id: Uuid::new_v4(),
        sources: vec![],
        eval_scores: None,
        cost_usd: None,
    }
}

// --- list_summaries handler ---

#[tokio::test]
async fn list_summaries_handler_returns_summaries() {
    let region_id = Uuid::new_v4();
    let svc = Arc::new(FakeServices::new());
    *svc.list_summaries.lock().unwrap() = Some(Ok(vec![sample_region_summary(region_id)]));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/regions/{region_id}/summaries")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert!(body["summaries"].is_array());
    assert_eq!(body["summaries"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn list_summaries_handler_propagates_service_error_as_500() {
    let region_id = Uuid::new_v4();
    let svc = Arc::new(FakeServices::new());
    *svc.list_summaries.lock().unwrap() = Some(Err(FakeErr("db down".to_string())));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/regions/{region_id}/summaries")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = read_body_json(resp).await;
    assert!(body["error"].as_str().unwrap().contains("db down"));
}

// --- get_config (happy path) ---

#[tokio::test]
async fn get_config_returns_all_entries() {
    let svc = Arc::new(FakeServices::new());
    *svc.get_config.lock().unwrap() = Some(Ok(vec![ConfigEntry {
        key: "chat_model".to_string(),
        value: "gpt-4".to_string(),
        description: Some("Primary chat model".to_string()),
        updated_at: chrono::Utc::now(),
    }]));

    let app = router(svc);
    let resp = app.oneshot(get("/orch/api/config")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["key"], "chat_model");
    assert_eq!(arr[0]["value"], "gpt-4");
}

// --- update_config (PATCH) ---

#[tokio::test]
async fn update_config_patch_forwards_body_and_returns_result() {
    let svc = Arc::new(FakeServices::new());
    *svc.update_config_result.lock().unwrap() = Some(Ok(vec![ConfigEntry {
        key: "chat_model".to_string(),
        value: "claude".to_string(),
        description: None,
        updated_at: chrono::Utc::now(),
    }]));

    let app = router(svc.clone());
    let req_body = serde_json::json!([
        {"key": "chat_model", "value": "claude"},
        {"key": "other", "value": "v"},
    ]);
    let resp = app
        .oneshot(patch_json("/orch/api/config", req_body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body[0]["key"], "chat_model");
    assert_eq!(body[0]["value"], "claude");

    let last = svc.last_update_config.lock().unwrap().clone().unwrap();
    assert_eq!(last.len(), 2);
    assert_eq!(last[0].key, "chat_model");
    assert_eq!(last[1].key, "other");
}

#[tokio::test]
async fn update_config_patch_rejects_malformed_body_with_400() {
    let svc = Arc::new(FakeServices::new());
    let app = router(svc);

    let req = Request::builder()
        .method(Method::PATCH)
        .uri("/orch/api/config")
        .header("content-type", "application/json")
        .body(Body::from("not json"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// --- get_all_regions ---

#[tokio::test]
async fn get_all_regions_returns_list() {
    let svc = Arc::new(FakeServices::new());
    *svc.all_regions.lock().unwrap() = Some(Ok(vec![Region {
        id: Uuid::new_v4(),
        region_id: 42,
        name: "Hippocampus".to_string(),
        acronym: Some("HPF".to_string()),
        color: None,
        structure_order: None,
        parent_region_id: None,
        parent_acronym: None,
    }]));

    let app = router(svc);
    let resp = app.oneshot(get("/orch/api/regions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["name"], "Hippocampus");
    assert_eq!(body[0]["region_id"], 42);
}

// --- get_active_batch_id ---

#[tokio::test]
async fn get_active_batch_handler_returns_batch_id_when_active() {
    let region_id = Uuid::new_v4();
    let batch = sample_batch(region_id, BatchStatus::Collecting, vec![1, 2, 3]);
    let batch_id = batch.id;

    let svc = Arc::new(FakeServices::new());
    *svc.active_batch.lock().unwrap() = Some(Ok(Some(batch)));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/regions/{region_id}/active-batch")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["region_id"], region_id.to_string());
    assert_eq!(body["active_batch_id"], batch_id.to_string());
}

#[tokio::test]
async fn get_active_batch_handler_returns_null_when_none() {
    let region_id = Uuid::new_v4();
    let svc = Arc::new(FakeServices::new());
    *svc.active_batch.lock().unwrap() = Some(Ok(None));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/regions/{region_id}/active-batch")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert!(body["active_batch_id"].is_null());
}

// --- get_region_status: covers app::get_region_status flow ---

#[tokio::test]
async fn region_status_done_when_summaries_present() {
    let region_id = Uuid::new_v4();
    let svc = Arc::new(FakeServices::new());
    *svc.active_batch.lock().unwrap() = Some(Ok(None));
    *svc.list_summaries.lock().unwrap() = Some(Ok(vec![sample_region_summary(region_id)]));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/regions/{region_id}/status")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["status"], "Done");
    assert_eq!(body["summary_count"], 1);
}

#[tokio::test]
async fn region_status_not_started_when_no_batch_no_summary() {
    let region_id = Uuid::new_v4();
    let svc = Arc::new(FakeServices::new());
    *svc.active_batch.lock().unwrap() = Some(Ok(None));
    *svc.list_summaries.lock().unwrap() = Some(Ok(vec![]));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/regions/{region_id}/status")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["status"], "NotStarted");
    assert_eq!(body["summary_count"], 0);
}

#[tokio::test]
async fn region_status_reflects_active_batch_collecting() {
    let region_id = Uuid::new_v4();
    let batch = sample_batch(region_id, BatchStatus::Collecting, vec![1]);
    let svc = Arc::new(FakeServices::new());
    *svc.active_batch.lock().unwrap() = Some(Ok(Some(batch)));
    *svc.list_summaries.lock().unwrap() = Some(Ok(vec![]));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/regions/{region_id}/status")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["status"], "FetchQueued");
}

// --- get_pipeline_stats: covers app::get_pipeline_stats aggregation ---

#[tokio::test]
async fn pipeline_stats_aggregates_latest_batch_per_region_across_statuses() {
    let r1 = Uuid::new_v4();
    let r2 = Uuid::new_v4();
    let r3 = Uuid::new_v4();

    let svc = Arc::new(FakeServices::new());
    *svc.total_regions.lock().unwrap() = Some(Ok(1000));
    *svc.regions_without_batches.lock().unwrap() = Some(Ok(900));
    *svc.actively_fetching_regions.lock().unwrap() = Some(Ok(7));

    // One batch per region, each in a different status.
    let batches = vec![
        (
            BatchStatus::Collecting,
            vec![sample_batch(r1, BatchStatus::Collecting, vec![])],
        ),
        (
            BatchStatus::Ready,
            vec![sample_batch(r2, BatchStatus::Ready, vec![])],
        ),
        (
            BatchStatus::Completed,
            vec![sample_batch(r3, BatchStatus::Completed, vec![])],
        ),
        (BatchStatus::Processing, vec![]),
        (BatchStatus::Failed, vec![]),
        (BatchStatus::Invalidated, vec![]),
    ];
    *svc.batches_by_status.lock().unwrap() = Some(batches);

    let app = router(svc);
    let resp = app.oneshot(get("/orch/api/pipeline/stats")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_body_json(resp).await;
    assert_eq!(body["total_regions"], 1000);
    assert_eq!(body["not_started"], 900);
    assert_eq!(body["fetching"], 7);
    assert_eq!(body["fetch_queued"], 1);
    assert_eq!(body["llm_queued"], 1);
    assert_eq!(body["done"], 1);
    assert_eq!(body["processing"], 0);
    assert_eq!(body["fetch_failed"], 0);
    assert_eq!(body["invalidated"], 0);
}

#[tokio::test]
async fn pipeline_stats_keeps_only_most_recent_batch_per_region() {
    // Same region_id in two different statuses → later (created_at) wins.
    let region_id = Uuid::new_v4();

    let old = ProcessingBatch {
        id: Uuid::new_v4(),
        region_id,
        status: BatchStatus::Failed,
        fetch_task_ids: vec![],
        expected_task_count: 0,
        content_hash: None,
        created_at: chrono::Utc::now() - chrono::Duration::hours(2),
        ready_at: None,
        processing_started_at: None,
        completed_at: None,
        summary_id: None,
        error_message: None,
    };
    let new = ProcessingBatch {
        id: Uuid::new_v4(),
        region_id,
        status: BatchStatus::Completed,
        fetch_task_ids: vec![],
        expected_task_count: 0,
        content_hash: None,
        created_at: chrono::Utc::now(),
        ready_at: None,
        processing_started_at: None,
        completed_at: None,
        summary_id: None,
        error_message: None,
    };

    let svc = Arc::new(FakeServices::new());
    *svc.total_regions.lock().unwrap() = Some(Ok(1));
    *svc.regions_without_batches.lock().unwrap() = Some(Ok(0));
    *svc.actively_fetching_regions.lock().unwrap() = Some(Ok(0));
    *svc.batches_by_status.lock().unwrap() = Some(vec![
        (BatchStatus::Failed, vec![old]),
        (BatchStatus::Completed, vec![new]),
        (BatchStatus::Collecting, vec![]),
        (BatchStatus::Ready, vec![]),
        (BatchStatus::Processing, vec![]),
        (BatchStatus::Invalidated, vec![]),
    ]);

    let app = router(svc);
    let resp = app.oneshot(get("/orch/api/pipeline/stats")).await.unwrap();
    let body = read_body_json(resp).await;
    // Only the newer batch (Completed) should count.
    assert_eq!(body["done"], 1);
    assert_eq!(body["fetch_failed"], 0);
}

// --- get_batch_status: covers app::get_batch_status + count_completed_tasks ---

#[tokio::test]
async fn get_batch_status_collecting_with_task_progress() {
    let region_id = Uuid::new_v4();
    let mut batch = sample_batch(region_id, BatchStatus::Collecting, vec![1, 2, 3]);
    batch.expected_task_count = 3;
    let batch_id = batch.id;

    let svc = Arc::new(FakeServices::new());
    *svc.batch_by_id.lock().unwrap() = Some(Ok(Some(batch)));
    *svc.count_completed_tasks.lock().unwrap() = Some(Ok(2));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/batches/{batch_id}/status")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["status"], "Fetching");
    assert_eq!(body["expected_tasks"], 3);
    assert_eq!(body["completed_tasks"], 2);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("Fetching papers")
    );
}

#[tokio::test]
async fn get_batch_status_failed_embeds_error_in_message() {
    let region_id = Uuid::new_v4();
    let mut batch = sample_batch(region_id, BatchStatus::Failed, vec![]);
    batch.error_message = Some("No papers found".to_string());
    let batch_id = batch.id;

    let svc = Arc::new(FakeServices::new());
    *svc.batch_by_id.lock().unwrap() = Some(Ok(Some(batch)));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/batches/{batch_id}/status")))
        .await
        .unwrap();
    let body = read_body_json(resp).await;
    assert_eq!(body["status"], "FetchFailed");
    assert_eq!(body["error"], "No papers found");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("No papers found")
    );
    // Empty fetch_task_ids → completed_tasks stays null.
    assert!(body["completed_tasks"].is_null());
}

#[tokio::test]
async fn get_batch_status_ready_has_waiting_message() {
    let region_id = Uuid::new_v4();
    let batch = sample_batch(region_id, BatchStatus::Ready, vec![1]);
    let batch_id = batch.id;

    let svc = Arc::new(FakeServices::new());
    *svc.batch_by_id.lock().unwrap() = Some(Ok(Some(batch)));
    *svc.count_completed_tasks.lock().unwrap() = Some(Ok(1));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/batches/{batch_id}/status")))
        .await
        .unwrap();
    let body = read_body_json(resp).await;
    assert_eq!(body["status"], "LlmQueued");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("waiting for LLM processing")
    );
}

// --- reverse_search ---

#[tokio::test]
async fn reverse_search_forwards_query_and_returns_results() {
    let svc = Arc::new(FakeServices::new());
    *svc.reverse_search_result.lock().unwrap() = Some(Ok(domain::SearchResponse {
        query: "memory".to_string(),
        results: vec![],
        total_found: 0,
    }));

    let app = router(svc.clone());
    let resp = app
        .oneshot(post_json(
            "/orch/api/search",
            serde_json::json!({"query": "memory"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["query"], "memory");
    assert_eq!(body["total_found"], 0);

    let observed = svc.last_reverse_query.lock().unwrap().clone();
    assert_eq!(observed.as_deref(), Some("memory"));
}

// --- worker endpoints ---

#[tokio::test]
async fn worker_status_endpoint_returns_list() {
    let svc = Arc::new(FakeServices::new());
    *svc.worker_status.lock().unwrap() = Some(Ok(vec![WorkerStatus {
        worker_id: "w-1".to_string(),
        status: "running".to_string(),
        current_task: None,
        tasks_processed: 3,
        started_at: 1700000000,
        worker_version: Some("v1".to_string()),
        last_heartbeat_at: Some(1700000100),
        uptime_seconds: 100.0,
        tasks_failed: 0,
        success_rate: 1.0,
        task_timeout_secs: 30,
        failure_backoff_base_secs: 5,
        max_retry_attempts: 3,
        backoff_strategy: "constant".to_string(),
    }]));

    let app = router(svc);
    let resp = app.oneshot(get("/orch/api/workers/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body[0]["worker_id"], "w-1");
    assert_eq!(body[0]["status"], "running");
}

#[tokio::test]
async fn allocate_workers_endpoint_forwards_body_and_returns_ids() {
    let svc = Arc::new(FakeServices::new());
    *svc.allocate_workers_result.lock().unwrap() = Some(Ok(WorkerAllocationResponse {
        success: true,
        worker_ids: vec!["w-1".to_string(), "w-2".to_string()],
        error_message: None,
    }));

    let app = router(svc.clone());
    let resp = app
        .oneshot(post_json(
            "/orch/api/workers/allocate",
            serde_json::json!({
                "worker_count": 2,
                "task_timeout_secs": 30,
                "max_retry_attempts": 3,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["success"], true);
    assert_eq!(body["worker_ids"].as_array().unwrap().len(), 2);

    let observed = svc.last_allocate_req.lock().unwrap().as_ref().map(|r| r.worker_count);
    assert_eq!(observed, Some(2));
}

#[tokio::test]
async fn stop_workers_endpoint_forwards_ids_and_returns_count() {
    let svc = Arc::new(FakeServices::new());
    *svc.stop_workers_result.lock().unwrap() = Some(Ok(WorkerStopResponse {
        success: true,
        workers_stopped: 1,
        error_message: None,
    }));

    let app = router(svc.clone());
    let resp = app
        .oneshot(post_json(
            "/orch/api/workers/stop",
            serde_json::json!({"worker_ids": ["w-1"]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["success"], true);
    assert_eq!(body["workers_stopped"], 1);

    let observed = svc.last_stop_req.lock().unwrap().as_ref().map(|r| r.worker_ids.clone());
    assert_eq!(observed, Some(vec!["w-1".to_string()]));
}

// --- chunk source ---

#[tokio::test]
async fn get_chunk_source_returns_details() {
    let chunk_id = Uuid::new_v4();
    let svc = Arc::new(FakeServices::new());
    *svc.chunk_source.lock().unwrap() = Some(Ok(domain::ChunkSourceResponse {
        chunk_id,
        chunk_text: "some text".to_string(),
        source_s3_key: Some("s3://k".to_string()),
        source_pmc_id: Some("PMC1".to_string()),
        source_uid: None,
        source_query: Some("q".to_string()),
        char_start: Some(0),
        char_end: Some(9),
    }));

    let app = router(svc);
    let resp = app
        .oneshot(get(&format!("/orch/api/chunks/{chunk_id}/source")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["chunk_text"], "some text");
    assert_eq!(body["source_pmc_id"], "PMC1");
}

// --- pipeline status ---

#[tokio::test]
async fn get_pipeline_status_aggregates_counts_and_running_workers() {
    let svc = Arc::new(FakeServices::new());
    *svc.pending_fetch_task_count.lock().unwrap() = Some(Ok(42));
    *svc.regions_without_queries_count.lock().unwrap() = Some(Ok(10));
    *svc.regions_with_queries_count.lock().unwrap() = Some(Ok(1190));
    // Two workers: one "running", one "idle" → only running counted.
    *svc.worker_status.lock().unwrap() = Some(Ok(vec![
        WorkerStatus {
            worker_id: "w-1".to_string(),
            status: "running".to_string(),
            current_task: None,
            tasks_processed: 0,
            started_at: 0,
            worker_version: None,
            last_heartbeat_at: None,
            uptime_seconds: 0.0,
            tasks_failed: 0,
            success_rate: 1.0,
            task_timeout_secs: 30,
            failure_backoff_base_secs: 5,
            max_retry_attempts: 3,
            backoff_strategy: "constant".to_string(),
        },
        WorkerStatus {
            worker_id: "w-2".to_string(),
            status: "idle".to_string(),
            current_task: None,
            tasks_processed: 0,
            started_at: 0,
            worker_version: None,
            last_heartbeat_at: None,
            uptime_seconds: 0.0,
            tasks_failed: 0,
            success_rate: 1.0,
            task_timeout_secs: 30,
            failure_backoff_base_secs: 5,
            max_retry_attempts: 3,
            backoff_strategy: "constant".to_string(),
        },
    ]));

    let app = router(svc);
    let resp = app.oneshot(get("/orch/api/pipeline/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["pending_fetch_tasks"], 42);
    assert_eq!(body["regions_without_queries"], 10);
    assert_eq!(body["regions_with_queries"], 1190);
    assert_eq!(body["worker_count"], 1);
}

#[tokio::test]
async fn get_pipeline_status_falls_back_to_zero_on_errors() {
    // app::get_pipeline_status uses unwrap_or(0) on every sub-call so even
    // when each underlying service errors the endpoint still returns 200.
    let svc = Arc::new(FakeServices::new());
    *svc.pending_fetch_task_count.lock().unwrap() = Some(Err(FakeErr("db".to_string())));
    *svc.regions_without_queries_count.lock().unwrap() = Some(Err(FakeErr("db".to_string())));
    *svc.regions_with_queries_count.lock().unwrap() = Some(Err(FakeErr("db".to_string())));
    *svc.worker_status.lock().unwrap() = Some(Err(FakeErr("fetcher down".to_string())));

    let app = router(svc);
    let resp = app.oneshot(get("/orch/api/pipeline/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["pending_fetch_tasks"], 0);
    assert_eq!(body["regions_without_queries"], 0);
    assert_eq!(body["regions_with_queries"], 0);
    assert_eq!(body["worker_count"], 0);
}

// --- generate_summary: existing active batch returns "already_in_progress" ---

#[tokio::test]
async fn generate_summary_returns_existing_batch_when_active() {
    let region_id = Uuid::new_v4();
    let batch = sample_batch(region_id, BatchStatus::Processing, vec![10, 20, 30]);
    let batch_id = batch.id;

    let svc = Arc::new(FakeServices::new());
    *svc.active_batch.lock().unwrap() = Some(Ok(Some(batch)));

    let app = router(svc);
    let post_req = Request::builder()
        .method(Method::POST)
        .uri(format!("/orch/api/regions/{region_id}/generate"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(post_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_json(resp).await;
    assert_eq!(body["batch_id"], batch_id.to_string());
    assert_eq!(body["already_in_progress"], true);
    assert_eq!(body["task_count"], 3);
    assert_eq!(body["query_count"], 0);
}

