use api::{ApiError, BrainAtlasApi, BrainRegionApi};
use app::AppError;
use axum::extract::State;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use domain::rpc_types::{ProcessRegionRequest, SearchBrainRegionRequest, StatusRequest};
use infra::{BrainAtlasInfra, InfraError};
use services::{BrainAtlasServices, ServiceError};
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

pub struct BrainAtlasServer {
    api: Arc<BrainAtlasApi<BrainAtlasServices<BrainAtlasInfra>>>,
}

impl Clone for BrainAtlasServer {
    fn clone(&self) -> Self {
        Self {
            api: self.api.clone(),
        }
    }
}

impl BrainAtlasServer {
    pub fn new(api: Arc<BrainAtlasApi<BrainAtlasServices<BrainAtlasInfra>>>) -> Self {
        Self { api }
    }

    pub fn into_router(self) -> Router {
        let cors = cors_layer();

        let api_routes = Router::new()
            .route("/health", get(health_handler))
            .route("/api/list", get(list_brain_regions_handler))
            .route("/api/search", post(search_brain_region_handler))
            .route("/api/status", post(status_handler))
            .route("/api/process", post(process_region_handler))
            .layer(cors)
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                    .on_response(DefaultOnResponse::new().level(Level::INFO)),
            )
            .with_state(self);

        Router::new().nest("/brainatlas-be", api_routes)
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

/// GET /brainatlas-be/api/list
async fn list_brain_regions_handler(
    State(server): State<BrainAtlasServer>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = server.api.list_brain_regions().await.map_err(ServerError)?;
    Ok(Json(resp))
}

/// POST /brainatlas-be/api/search  body: { "id": { "value": "<uuid>" } }
async fn search_brain_region_handler(
    State(server): State<BrainAtlasServer>,
    Json(body): Json<SearchBrainRegionRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let id = body.id.and_then(|u| u.value.parse::<uuid::Uuid>().ok());
    let resp = server.api.search_brain_region(id).await.map_err(ServerError)?;
    Ok(Json(resp))
}

/// POST /brainatlas-be/api/status  body: { "id": { "value": "<uuid>" } }
async fn status_handler(
    State(server): State<BrainAtlasServer>,
    Json(body): Json<StatusRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let id = body
        .id
        .and_then(|u| u.value.parse::<uuid::Uuid>().ok())
        .ok_or(ServerError(Error::MissingOrInvalidId))?;
    let resp = server.api.status(id).await.map_err(ServerError)?;
    Ok(Json(resp))
}

/// POST /brainatlas-be/api/process  body: { "region_id": { "value": "<uuid>" }, "s3_keys": ["..."] }
async fn process_region_handler(
    State(server): State<BrainAtlasServer>,
    Json(body): Json<ProcessRegionRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let region_id = body
        .region_id
        .and_then(|u| u.value.parse::<uuid::Uuid>().ok())
        .ok_or(ServerError(Error::MissingOrInvalidId))?;
    let resp = server.api.process_region(region_id, body.s3_keys).await.map_err(ServerError)?;
    Ok(Json(resp))
}
