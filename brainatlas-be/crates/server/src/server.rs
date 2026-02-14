use api::BrainRegionApi;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

pub struct BrainAtlasServer<A> {
    api: Arc<A>,
}

impl<A> Clone for BrainAtlasServer<A> {
    fn clone(&self) -> Self {
        Self {
            api: self.api.clone(),
        }
    }
}

impl<A: BrainRegionApi + Send + Sync + 'static> BrainAtlasServer<A> {
    pub fn new(api: Arc<A>) -> Self {
        Self { api }
    }

    pub fn into_router(self) -> Router {
        let cors = cors_layer();

        let api_routes = Router::new()
            .route("/health", get(health_handler))
            .route("/api/list", get(list_brain_regions_handler::<A>))
            .route("/api/search/{id}", get(search_brain_region_handler::<A>))
            .route("/api/status/{id}", get(status_handler::<A>))
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

/// Builds a `CorsLayer` from the `CORS_ORIGIN` environment variable.
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

struct ServerError(Box<dyn std::error::Error>);

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.0.to_string() });
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}

impl<E: std::error::Error + 'static> From<E> for ServerError {
    fn from(e: E) -> Self {
        ServerError(Box::new(e))
    }
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

/// GET /brainatlas-be/api/list
async fn list_brain_regions_handler<A>(
    State(server): State<BrainAtlasServer<A>>,
) -> Result<impl IntoResponse, ServerError>
where
    A: BrainRegionApi + Send + Sync + 'static,
{
    let regions = server.api.list_brain_regions().await?;
    Ok(Json(regions))
}

/// GET /brainatlas-be/api/search/:id
async fn search_brain_region_handler<A>(
    State(server): State<BrainAtlasServer<A>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError>
where
    A: BrainRegionApi + Send + Sync + 'static,
{
    let entry = server.api.search_brain_region(id).await?;
    Ok(Json(entry))
}

/// GET /brainatlas-be/api/status/:id
async fn status_handler<A>(
    State(server): State<BrainAtlasServer<A>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError>
where
    A: BrainRegionApi + Send + Sync + 'static,
{
    let status = server.api.status(id).await?;
    Ok(Json(status))
}
