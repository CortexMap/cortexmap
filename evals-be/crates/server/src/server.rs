use api::{ApiError, Evals, EvalsApi};
use app::AppError;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use infra::{EvalsInfra, InfraError};
use rpc_types::ScoreRequest;
use serde::Deserialize;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

type Error = ApiError<AppError<InfraError>>;

struct ServerError(Error);

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self.0 {
            Error::MissingOrInvalidId => (StatusCode::BAD_REQUEST, self.0.to_string()),
            Error::AppError(AppError::SummaryNotFound) => {
                (StatusCode::NOT_FOUND, "summary not found".to_string())
            }
            Error::AppError(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl From<Error> for ServerError {
    fn from(e: Error) -> Self {
        ServerError(e)
    }
}

pub struct EvalsServer {
    api: Arc<Evals<EvalsInfra, EvalsInfra, EvalsInfra, InfraError>>,
}

impl Clone for EvalsServer {
    fn clone(&self) -> Self {
        Self {
            api: self.api.clone(),
        }
    }
}

impl EvalsServer {
    pub fn new(api: Arc<Evals<EvalsInfra, EvalsInfra, EvalsInfra, InfraError>>) -> Self {
        Self { api }
    }

    pub fn into_router(self) -> Router {
        let cors = cors_layer();

        let routes = Router::new()
            .route("/health", get(health))
            .route("/api/evals/score", post(score))
            .route("/api/evals/scores/{summary_id}", get(scores_for_summary))
            .route("/api/evals/summary", get(aggregate_summary))
            .route("/api/evals/worst", get(worst_offenders))
            .route("/api/evals/unscored", get(unscored))
            .layer(cors)
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                    .on_response(DefaultOnResponse::new().level(Level::INFO)),
            )
            .with_state(self);

        Router::new().nest("/evals-be", routes)
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

async fn score(
    State(server): State<EvalsServer>,
    Json(req): Json<ScoreRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = server.api.score(req).await?;
    Ok(Json(resp))
}

async fn scores_for_summary(
    State(server): State<EvalsServer>,
    Path(summary_id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = server.api.scores_for_summary(summary_id).await?;
    Ok(Json(resp))
}

#[derive(Deserialize)]
struct AggregateQuery {
    eval_version: Option<String>,
}

async fn aggregate_summary(
    State(server): State<EvalsServer>,
    Query(q): Query<AggregateQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = server.api.aggregate_summary(q.eval_version).await?;
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

async fn worst_offenders(
    State(server): State<EvalsServer>,
    Query(q): Query<WorstQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = server
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

async fn unscored(
    State(server): State<EvalsServer>,
    Query(q): Query<UnscoredQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = server
        .api
        .list_unscored_summary_ids(q.eval_version, q.limit)
        .await?;
    Ok(Json(resp))
}
