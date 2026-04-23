//! Generic axum router builder.
//!
//! Builds the evals-be HTTP `Router` from any `EvalsApi` implementation whose
//! `Error` is `ApiError<AppError<E>>`. The production server wires
//! `Evals<EvalsInfra, EvalsInfra, InfraError>` through here; tests wire an
//! in-memory fake of the same trait so handlers can be exercised without a
//! real database or network.

use crate::api::EvalsApi;
use crate::error::ApiError;
use app::AppError;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rpc_types::{BatchEvalRequest, InitScoreRequest, StepRequest};
use serde::Deserialize;
use std::error::Error;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

/// Build the evals-be axum `Router`. Nests all application routes under
/// `/evals-be/…` to match the production deployment.
///
/// `A` is any `EvalsApi` whose `Error` type is the standard
/// `ApiError<AppError<E>>` wrapper — this matches the `Evals<…>` struct in
/// `api.rs` and keeps the error-to-HTTP mapping in one place.
pub fn build_router<A, E>(api: Arc<A>) -> Router
where
    A: EvalsApi<Error = ApiError<AppError<E>>> + 'static,
    E: Error + Send + Sync + 'static,
{
    let state = AppState { api };

    let routes = Router::new()
        .route("/health", get(health))
        .route("/api/evals/score/init", post(init_score::<A, E>))
        .route("/api/evals/batch", post(batch_eval::<A, E>))
        .route("/api/evals/score/step", post(step_score::<A, E>))
        .route(
            "/api/evals/scores/{summary_id}",
            get(scores_for_summary::<A, E>),
        )
        .route("/api/evals/summary", get(aggregate_summary::<A, E>))
        .route("/api/evals/worst", get(worst_offenders::<A, E>))
        .route("/api/evals/unscored", get(unscored::<A, E>))
        .layer(cors_layer())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(state);

    Router::new().nest("/evals-be", routes)
}

/// Shared state held by every handler: a reference-counted handle to the
/// trait object implementing `EvalsApi`.
struct AppState<A>
where
    A: EvalsApi + 'static,
{
    api: Arc<A>,
}

// `#[derive(Clone)]` would require `A: Clone`; we only need the `Arc`
// cloned, so do it manually.
impl<A> Clone for AppState<A>
where
    A: EvalsApi + 'static,
{
    fn clone(&self) -> Self {
        Self {
            api: self.api.clone(),
        }
    }
}

/// Wraps any `ApiError<AppError<E>>` so it can be converted into an axum
/// response with the right HTTP status code.
struct ServerError<E: Error + Send + Sync + 'static>(ApiError<AppError<E>>);

impl<E> From<ApiError<AppError<E>>> for ServerError<E>
where
    E: Error + Send + Sync + 'static,
{
    fn from(e: ApiError<AppError<E>>) -> Self {
        ServerError(e)
    }
}

impl<E> IntoResponse for ServerError<E>
where
    E: Error + Send + Sync + 'static,
{
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

fn cors_layer() -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
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

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn init_score<A, E>(
    State(state): State<AppState<A>>,
    Json(req): Json<InitScoreRequest>,
) -> Result<Response, ServerError<E>>
where
    A: EvalsApi<Error = ApiError<AppError<E>>> + 'static,
    E: Error + Send + Sync + 'static,
{
    let resp = state.api.init_score(req).await?;
    Ok(Json(resp).into_response())
}

async fn batch_eval<A, E>(
    State(state): State<AppState<A>>,
    Json(req): Json<BatchEvalRequest>,
) -> Result<Response, ServerError<E>>
where
    A: EvalsApi<Error = ApiError<AppError<E>>> + 'static,
    E: Error + Send + Sync + 'static,
{
    let resp = state.api.batch_eval(req).await?;
    Ok((StatusCode::ACCEPTED, Json(resp)).into_response())
}

async fn step_score<A, E>(
    State(state): State<AppState<A>>,
    Json(req): Json<StepRequest>,
) -> Result<Response, ServerError<E>>
where
    A: EvalsApi<Error = ApiError<AppError<E>>> + 'static,
    E: Error + Send + Sync + 'static,
{
    let resp = state.api.step_score(req).await?;
    Ok(Json(resp).into_response())
}

async fn scores_for_summary<A, E>(
    State(state): State<AppState<A>>,
    Path(summary_id): Path<uuid::Uuid>,
) -> Result<Response, ServerError<E>>
where
    A: EvalsApi<Error = ApiError<AppError<E>>> + 'static,
    E: Error + Send + Sync + 'static,
{
    let resp = state.api.scores_for_summary(summary_id).await?;
    Ok(Json(resp).into_response())
}

#[derive(Deserialize)]
struct AggregateQuery {
    eval_version: Option<String>,
}

async fn aggregate_summary<A, E>(
    State(state): State<AppState<A>>,
    Query(q): Query<AggregateQuery>,
) -> Result<Response, ServerError<E>>
where
    A: EvalsApi<Error = ApiError<AppError<E>>> + 'static,
    E: Error + Send + Sync + 'static,
{
    let resp = state.api.aggregate_summary(q.eval_version).await?;
    Ok(Json(resp).into_response())
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

async fn worst_offenders<A, E>(
    State(state): State<AppState<A>>,
    Query(q): Query<WorstQuery>,
) -> Result<Response, ServerError<E>>
where
    A: EvalsApi<Error = ApiError<AppError<E>>> + 'static,
    E: Error + Send + Sync + 'static,
{
    let resp = state
        .api
        .worst_offenders(q.metric, q.limit, q.eval_version)
        .await?;
    Ok(Json(resp).into_response())
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

async fn unscored<A, E>(
    State(state): State<AppState<A>>,
    Query(q): Query<UnscoredQuery>,
) -> Result<Response, ServerError<E>>
where
    A: EvalsApi<Error = ApiError<AppError<E>>> + 'static,
    E: Error + Send + Sync + 'static,
{
    let resp = state
        .api
        .list_unscored_summary_ids(q.eval_version, q.limit)
        .await?;
    Ok(Json(resp).into_response())
}
