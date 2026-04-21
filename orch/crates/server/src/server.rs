use api::{ApiError, Orch, OrchApi};
use app::{AppError, Services};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

// Generic server that owns an Arc<Orch<S>>. The concrete binary uses the
// production services; tests can substitute an in-memory `Services` impl.
pub struct OrchServer<S> {
    pub api: Arc<Orch<S>>,
}

impl<S> Clone for OrchServer<S> {
    fn clone(&self) -> Self {
        Self {
            api: self.api.clone(),
        }
    }
}

impl<S> OrchServer<S> {
    pub fn new(api: Arc<Orch<S>>) -> Self {
        Self { api }
    }
}

/// Wrap an `OrchApi::Error` so we can implement `IntoResponse` without
/// orphan-rules trouble.
struct ServerError<E: std::error::Error + Send + Sync + 'static>(ApiError<AppError<E>>);

impl<E: std::error::Error + Send + Sync + 'static> IntoResponse for ServerError<E> {
    fn into_response(self) -> Response {
        let (status, msg) = match &self.0 {
            ApiError::MissingOrInvalidId => (StatusCode::BAD_REQUEST, self.0.to_string()),
            ApiError::NotImplemented => (StatusCode::NOT_IMPLEMENTED, self.0.to_string()),
            ApiError::AppError(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl<E: std::error::Error + Send + Sync + 'static> From<ApiError<AppError<E>>> for ServerError<E> {
    fn from(e: ApiError<AppError<E>>) -> Self {
        ServerError(e)
    }
}

impl<E, S> OrchServer<S>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    pub fn into_router(self) -> Router {
        let cors = cors_layer();

        let api_routes = Router::new()
            .route("/health", get(health_handler))
            .route("/api/regions", get(get_all_regions_handler::<E, S>))
            .route(
                "/api/regions/{id}/summaries",
                get(list_summaries_handler::<E, S>),
            )
            .route(
                "/api/regions/{id}/generate",
                post(generate_summary_handler::<E, S>),
            )
            .route(
                "/api/regions/{id}/active-batch",
                get(get_active_batch_handler::<E, S>),
            )
            .route("/api/search", post(reverse_search_handler::<E, S>))
            .route(
                "/api/batches/{id}/status",
                get(get_batch_status_handler::<E, S>),
            )
            .route(
                "/api/regions/{id}/status",
                get(get_region_status_handler::<E, S>),
            )
            .route(
                "/api/pipeline/stats",
                get(get_pipeline_stats_handler::<E, S>),
            )
            .route("/api/config", get(get_config_handler::<E, S>))
            .route("/api/config", patch(update_config_handler::<E, S>))
            .route(
                "/api/chunks/{chunk_id}/source",
                get(get_chunk_source_handler::<E, S>),
            )
            .route(
                "/api/workers/status",
                get(get_worker_status_handler::<E, S>),
            )
            .route(
                "/api/workers/allocate",
                post(allocate_workers_handler::<E, S>),
            )
            .route("/api/workers/stop", post(stop_workers_handler::<E, S>))
            .route(
                "/api/pipeline/status",
                get(get_pipeline_status_handler::<E, S>),
            )
            .route(
                "/api/pipeline/trigger",
                post(trigger_pipeline_handler::<E, S>),
            )
            .route("/dev/stats", get(dev_stats_page_handler))
            .route(
                "/dev/api/system-stats",
                get(dev_system_stats_handler::<E, S>),
            )
            .route(
                "/dev/api/summary-freshness",
                get(dev_summary_freshness_handler::<E, S>),
            )
            .route("/dev/api/redis-stats", get(dev_redis_stats_handler::<E, S>))
            .route("/api/evals/status", get(get_eval_status_handler::<E, S>))
            .route("/api/evals/worst", get(get_eval_worst_handler::<E, S>))
            .route(
                "/api/evals/runs/{run_id}/cost",
                get(get_eval_run_cost_handler::<E, S>),
            )
            .layer(cors)
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                    .on_response(DefaultOnResponse::new().level(Level::INFO)),
            )
            .with_state(self);

        Router::new().nest("/orch", api_routes)
    }
}

fn cors_layer() -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::OPTIONS])
        .allow_headers(tower_http::cors::Any);

    match std::env::var("CORS_ORIGIN") {
        Ok(origins) if origins != "*" => {
            let allowed: Vec<HeaderValue> = origins
                .split(',')
                .filter_map(|o| o.trim().parse::<HeaderValue>().ok())
                .collect();
            layer.allow_origin(AllowOrigin::list(allowed))
        }
        _ => layer.allow_origin(tower_http::cors::Any),
    }
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn list_summaries_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.list_summaries(id).await?;
    Ok(Json(result))
}

async fn generate_summary_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.generate_summary(id).await?;
    Ok(Json(result))
}

async fn get_batch_status_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_batch_status(id).await?;
    Ok(Json(result))
}

async fn get_active_batch_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_active_batch(id).await?;
    Ok(Json(serde_json::json!({
        "region_id": id,
        "active_batch_id": result,
    })))
}

async fn get_region_status_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_region_status(id).await?;
    Ok(Json(result))
}

async fn get_pipeline_stats_handler<E, S>(
    State(server): State<OrchServer<S>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_pipeline_stats().await?;
    Ok(Json(result))
}

async fn get_config_handler<E, S>(
    State(server): State<OrchServer<S>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_config().await?;
    Ok(Json(result))
}

async fn update_config_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Json(body): Json<Vec<domain::ConfigEntryUpdate>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.update_config(body).await?;
    Ok(Json(result))
}

async fn get_all_regions_handler<E, S>(
    State(server): State<OrchServer<S>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_all_regions().await?;
    Ok(Json(result))
}

async fn get_chunk_source_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Path(chunk_id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_chunk_source(chunk_id).await?;
    Ok(Json(result))
}

async fn get_worker_status_handler<E, S>(
    State(server): State<OrchServer<S>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_worker_status().await?;
    Ok(Json(result))
}

async fn allocate_workers_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Json(body): Json<domain::AllocateWorkersRequest>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.allocate_workers(body).await?;
    Ok(Json(result))
}

async fn stop_workers_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Json(body): Json<domain::StopWorkersRequest>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.stop_workers(body).await?;
    Ok(Json(result))
}

async fn get_pipeline_status_handler<E, S>(
    State(server): State<OrchServer<S>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_pipeline_status().await?;
    Ok(Json(result))
}

async fn reverse_search_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Json(body): Json<domain::SearchRequest>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.reverse_search(body.query).await?;
    Ok(Json(result))
}

async fn dev_system_stats_handler<E, S>(
    State(server): State<OrchServer<S>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_system_stats().await?;
    Ok(Json(result))
}

async fn dev_summary_freshness_handler<E, S>(
    State(server): State<OrchServer<S>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_summary_freshness().await?;
    Ok(Json(result))
}

async fn dev_stats_page_handler() -> impl IntoResponse {
    axum::response::Html(include_str!("dev_stats.html"))
}

async fn trigger_pipeline_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Json(body): Json<domain::PipelineTriggerRequest>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.trigger_pipeline(body).await?;
    Ok(Json(result))
}

async fn dev_redis_stats_handler<E, S>(
    State(server): State<OrchServer<S>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_redis_stats().await?;
    Ok(Json(result))
}

async fn get_eval_status_handler<E, S>(
    State(server): State<OrchServer<S>>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_eval_status().await?;
    Ok(Json(result))
}

#[derive(serde::Deserialize)]
struct WorstQuery {
    metric: Option<String>,
    limit: Option<u32>,
}

async fn get_eval_worst_handler<E, S>(
    State(server): State<OrchServer<S>>,
    axum::extract::Query(q): axum::extract::Query<WorstQuery>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let metric = q.metric.unwrap_or_else(|| "groundedness".to_string());
    let limit = q.limit.unwrap_or(10) as i64;
    let result = server.api.get_eval_worst(metric, limit).await?;
    Ok(Json(result))
}

async fn get_eval_run_cost_handler<E, S>(
    State(server): State<OrchServer<S>>,
    Path(run_id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    let result = server.api.get_eval_run_cost(run_id).await?;
    Ok(Json(result))
}
