use api::{ApiError, BrainAtlasApi, BrainRegionApi};
use app::AppError;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use domain::rpc_types::evals::{
    EmbedRequest, EmbedResponse, ExtractClaimsRequest, JudgeGroundednessRequest,
    JudgeRubricRequest, UsageAggregateQuery,
};
use domain::rpc_types::{
    GenerateQueriesRequest, ProcessRegionRequest, SearchBrainRegionRequest, StatusRequest,
};
use domain::{UsageAggregateFilter};
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

    pub fn into_router(self, cors_origin: Option<String>) -> Router {
        let cors = cors_layer(cors_origin);

        let api_routes = Router::new()
            .route("/health", get(health_handler))
            .route("/api/list", get(list_brain_regions_handler))
            .route("/api/search", post(search_brain_region_handler))
            .route("/api/status", post(status_handler))
            .route("/api/process", post(process_region_handler))
            .route("/api/generate-queries", post(generate_queries_handler))
            .route("/api/llm/embed", post(llm_embed_handler))
            .route("/api/llm/extract-claims", post(llm_extract_claims_handler))
            .route(
                "/api/llm/judge-groundedness",
                post(llm_judge_groundedness_handler),
            )
            .route("/api/llm/judge-rubric", post(llm_judge_rubric_handler))
            .route("/api/llm/usage", get(llm_usage_handler))
            .route(
                "/api/chunks/{chunk_id}/source",
                get(get_chunk_source_handler),
            )
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

fn cors_layer(cors_origin: Option<String>) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(tower_http::cors::Any);

    match cors_origin {
        Some(origins) if origins != "*" => {
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
    let resp = server
        .api
        .search_brain_region(id)
        .await
        .map_err(ServerError)?;
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

/// POST /brainatlas-be/api/process  body: { "region_id": { "value": "<uuid>" }, "batch_id": { "value": "<uuid>" }, "s3_keys": ["..."], "paper_metadata": [...] }
async fn process_region_handler(
    State(server): State<BrainAtlasServer>,
    Json(body): Json<ProcessRegionRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let region_id = body
        .region_id
        .and_then(|u| u.value.parse::<uuid::Uuid>().ok());
    let batch_id = body
        .batch_id
        .and_then(|u| u.value.parse::<uuid::Uuid>().ok());
    let resp = server
        .api
        .process_region(
            region_id,
            batch_id,
            body.s3_keys,
            body.paper_metadata,
            body.chat_model,
            body.embedding_model,
            body.skip_summarization.unwrap_or(false),
            body.correlation_id,
        )
        .await
        .map_err(ServerError)?;
    Ok(Json(resp))
}

/// POST /brainatlas-be/api/generate-queries  body: { "region_name": "hippocampus", "count": 3 }
async fn generate_queries_handler(
    State(server): State<BrainAtlasServer>,
    Json(body): Json<GenerateQueriesRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = server
        .api
        .generate_queries(body.region_name, body.count, body.correlation_id)
        .await
        .map_err(ServerError)?;
    Ok(Json(resp))
}

/// GET /brainatlas-be/api/chunks/{chunk_id}/source
async fn get_chunk_source_handler(
    State(server): State<BrainAtlasServer>,
    Path(chunk_id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError> {
    let resp = server
        .api
        .get_chunk_source(chunk_id)
        .await
        .map_err(ServerError)?;
    match resp {
        Some(source) => Ok(Json(serde_json::json!(source))),
        None => Err(ServerError(Error::MissingOrInvalidId)),
    }
}

/// Convert an `AppError` into a `ServerError` for the eval handlers (which
/// bypass the `BrainRegionApi` trait and so don't get the `ApiError` wrapper).
fn from_app_error(e: AppError<ServiceError<InfraError>>) -> ServerError {
    ServerError(Error::AppError(e))
}

/// POST /brainatlas-be/api/llm/embed
async fn llm_embed_handler(
    State(server): State<BrainAtlasServer>,
    Json(body): Json<EmbedRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let embedding = server
        .api
        .embed(
            &body.text,
            body.embedding_model.as_deref(),
            body.correlation_id,
        )
        .await
        .map_err(from_app_error)?;
    Ok(Json(EmbedResponse { embedding }))
}

/// POST /brainatlas-be/api/llm/extract-claims
async fn llm_extract_claims_handler(
    State(server): State<BrainAtlasServer>,
    Json(body): Json<ExtractClaimsRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let claims = server
        .api
        .extract_claims(
            &body.summary_text,
            &body.region_name,
            body.chat_model.as_deref(),
            body.correlation_id,
        )
        .await
        .map_err(from_app_error)?;
    Ok(Json(claims))
}

/// POST /brainatlas-be/api/llm/judge-groundedness
async fn llm_judge_groundedness_handler(
    State(server): State<BrainAtlasServer>,
    Json(body): Json<JudgeGroundednessRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let verdict = server
        .api
        .judge_groundedness(
            &body.claim_text,
            &body.evidence_chunks,
            body.chat_model.as_deref(),
            body.correlation_id,
        )
        .await
        .map_err(from_app_error)?;
    Ok(Json(verdict))
}

/// POST /brainatlas-be/api/llm/judge-rubric
async fn llm_judge_rubric_handler(
    State(server): State<BrainAtlasServer>,
    Json(body): Json<JudgeRubricRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let scores = server
        .api
        .judge_rubric(
            &body.summary_text,
            &body.region_name,
            body.chat_model.as_deref(),
            body.correlation_id,
        )
        .await
        .map_err(from_app_error)?;
    Ok(Json(scores))
}

/// GET /brainatlas-be/api/llm/usage?since=…&model=…&correlation_id=…
async fn llm_usage_handler(
    State(server): State<BrainAtlasServer>,
    Query(q): Query<UsageAggregateQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let parse_ts = |s: Option<String>| -> Result<Option<chrono::DateTime<chrono::Utc>>, ServerError> {
        match s {
            None => Ok(None),
            Some(v) => chrono::DateTime::parse_from_rfc3339(&v)
                .map(|d| Some(d.with_timezone(&chrono::Utc)))
                .map_err(|_| ServerError(Error::MissingOrInvalidId)),
        }
    };
    let parse_uuid = |s: Option<String>| -> Result<Option<uuid::Uuid>, ServerError> {
        match s {
            None => Ok(None),
            Some(v) => v
                .parse::<uuid::Uuid>()
                .map(Some)
                .map_err(|_| ServerError(Error::MissingOrInvalidId)),
        }
    };
    let filter = UsageAggregateFilter {
        since: parse_ts(q.since)?,
        until: parse_ts(q.until)?,
        model: q.model,
        correlation_id: q.correlation_id,
        correlation_id_prefix: q.correlation_id_prefix,
        region_id: q.region_id,
        summary_id: parse_uuid(q.summary_id)?,
        batch_id: parse_uuid(q.batch_id)?,
        caller_tag: q.caller_tag,
    };
    let agg = server
        .api
        .usage_aggregate(filter)
        .await
        .map_err(from_app_error)?;
    Ok(Json(agg))
}
