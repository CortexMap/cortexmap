use api::{ApiError, Orch, OrchApi};
use app::AppError;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use infra::{InfraError, OrchInfra};
use services::{OrchServices, ServiceError};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

// Concrete error chain — no generics needed in server.
type Error = ApiError<AppError<ServiceError<InfraError>>>;

struct ServerError(Error);

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self.0 {
            Error::MissingOrInvalidId => (StatusCode::BAD_REQUEST, self.0.to_string()),
            Error::NotImplemented => (StatusCode::NOT_IMPLEMENTED, self.0.to_string()),
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

pub struct OrchServer {
    pub api: Arc<Orch<OrchServices<OrchInfra>>>,
}

impl Clone for OrchServer {
    fn clone(&self) -> Self {
        Self {
            api: self.api.clone(),
        }
    }
}

impl OrchServer {
    pub fn new(api: Arc<Orch<OrchServices<OrchInfra>>>) -> Self {
        Self { api }
    }

    pub fn into_router(self) -> Router {
        let cors = cors_layer();

        let api_routes = Router::new()
            .route("/health", get(health_handler))
            .route("/api/regions", get(get_all_regions_handler))
            .route("/api/regions/{id}/summaries", get(list_summaries_handler))
            .route("/api/regions/{id}/generate", post(generate_summary_handler))
            .route("/api/batches/{id}/status", get(get_batch_status_handler))
            .route("/api/regions/{id}/status", get(get_region_status_handler))
            .route("/api/pipeline/stats", get(get_pipeline_stats_handler))
            .route("/api/config", get(get_config_handler))
            .route("/api/config", patch(update_config_handler))
            .route("/api/chunks/{chunk_id}/source", get(get_chunk_source_handler))
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

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn list_summaries_handler(
    State(server): State<OrchServer>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError> {
    let result = server.api.list_summaries(id).await?;
    Ok(Json(result))
}

async fn generate_summary_handler(
    State(server): State<OrchServer>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError> {
    let result = server.api.generate_summary(id).await?;
    Ok(Json(result))
}

async fn get_batch_status_handler(
    State(server): State<OrchServer>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError> {
    let result = server.api.get_batch_status(id).await?;
    Ok(Json(result))
}

async fn get_region_status_handler(
    State(server): State<OrchServer>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError> {
    let result = server.api.get_region_status(id).await?;
    Ok(Json(result))
}

async fn get_pipeline_stats_handler(
    State(server): State<OrchServer>,
) -> Result<impl IntoResponse, ServerError> {
    let result = server.api.get_pipeline_stats().await?;
    Ok(Json(result))
}

async fn get_config_handler(
    State(server): State<OrchServer>,
) -> Result<impl IntoResponse, ServerError> {
    let result = server.api.get_config().await?;
    Ok(Json(result))
}

async fn update_config_handler(
    State(server): State<OrchServer>,
    Json(body): Json<Vec<domain::ConfigEntryUpdate>>,
) -> Result<impl IntoResponse, ServerError> {
    let result = server.api.update_config(body).await?;
    Ok(Json(result))
}

async fn get_all_regions_handler(
    State(server): State<OrchServer>,
) -> Result<impl IntoResponse, ServerError> {
    let result = server.api.get_all_regions().await?;
    Ok(Json(result))
}

async fn get_chunk_source_handler(
    State(server): State<OrchServer>,
    Path(chunk_id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError> {
    let result = server.api.get_chunk_source(chunk_id).await?;
    Ok(Json(result))
}
