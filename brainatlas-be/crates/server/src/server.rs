use api::{ApiError, BrainAtlasApi, BrainRegionApi};
use app::{AppError, Services};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use domain::UsageAggregateFilter;
use domain::rpc_types::evals::{
    EmbedRequest, EmbedResponse, ExtractClaimsRequest, JudgeCitationRequest,
    JudgeGroundednessRequest, JudgeRubricRequest, UsageAggregateQuery,
};
use domain::rpc_types::{
    GenerateQueriesRequest, ProcessRegionRequest, SearchBrainRegionRequest, StatusRequest,
};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

// Concrete error chain wrapper used by handlers.
// `E` is the upstream service-layer error type (e.g. `ServiceError<InfraError>`
// in production, or a lightweight test error in handler unit tests).
type Error<E> = ApiError<AppError<E>>;

struct ServerError<E: std::error::Error + Send + Sync + 'static>(Error<E>);

impl<E: std::error::Error + Send + Sync + 'static> IntoResponse for ServerError<E> {
    fn into_response(self) -> Response {
        let (status, msg) = match &self.0 {
            Error::MissingOrInvalidId => (StatusCode::BAD_REQUEST, self.0.to_string()),
            Error::NotImplemented => (StatusCode::NOT_IMPLEMENTED, self.0.to_string()),
            Error::AppError(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl<E: std::error::Error + Send + Sync + 'static> From<Error<E>> for ServerError<E> {
    fn from(e: Error<E>) -> Self {
        ServerError(e)
    }
}

/// Generic axum server wrapper. The `S` parameter is the concrete `Services`
/// implementation (production uses `BrainAtlasServices<BrainAtlasInfra>`; tests
/// inject hand-rolled fakes).
pub struct BrainAtlasServer<S> {
    api: Arc<BrainAtlasApi<S>>,
}

impl<S> Clone for BrainAtlasServer<S> {
    fn clone(&self) -> Self {
        Self {
            api: self.api.clone(),
        }
    }
}

impl<S> BrainAtlasServer<S> {
    pub fn new(api: Arc<BrainAtlasApi<S>>) -> Self {
        Self { api }
    }
}

impl<E, S> BrainAtlasServer<S>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    pub fn into_router(self, cors_origin: Option<String>) -> Router {
        let cors = cors_layer(cors_origin);

        let api_routes = Router::new()
            .route("/health", get(health_handler))
            .route("/api/list", get(list_brain_regions_handler::<S>))
            .route("/api/search", post(search_brain_region_handler::<S>))
            .route("/api/status", post(status_handler::<S>))
            .route("/api/process", post(process_region_handler::<S>))
            .route("/api/generate-queries", post(generate_queries_handler::<S>))
            .route("/api/llm/embed", post(llm_embed_handler::<S>))
            .route(
                "/api/llm/extract-claims",
                post(llm_extract_claims_handler::<S>),
            )
            .route(
                "/api/llm/judge-groundedness",
                post(llm_judge_groundedness_handler::<S>),
            )
            .route(
                "/api/llm/judge-rubric",
                post(llm_judge_rubric_handler::<S>),
            )
            .route(
                "/api/llm/judge-citation",
                post(llm_judge_citation_handler::<S>),
            )
            .route("/api/llm/usage", get(llm_usage_handler::<S>))
            .route(
                "/api/chunks/{chunk_id}/source",
                get(get_chunk_source_handler::<S>),
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
async fn list_brain_regions_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
    let resp = server.api.list_brain_regions().await.map_err(ServerError)?;
    Ok(Json(resp))
}

/// POST /brainatlas-be/api/search  body: { "id": { "value": "<uuid>" } }
async fn search_brain_region_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Json(body): Json<SearchBrainRegionRequest>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
    let id = body.id.and_then(|u| u.value.parse::<uuid::Uuid>().ok());
    let resp = server
        .api
        .search_brain_region(id)
        .await
        .map_err(ServerError)?;
    Ok(Json(resp))
}

/// POST /brainatlas-be/api/status  body: { "id": { "value": "<uuid>" } }
async fn status_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Json(body): Json<StatusRequest>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
    let id = body
        .id
        .and_then(|u| u.value.parse::<uuid::Uuid>().ok())
        .ok_or(ServerError(Error::MissingOrInvalidId))?;
    let resp = server.api.status(id).await.map_err(ServerError)?;
    Ok(Json(resp))
}

/// POST /brainatlas-be/api/process  body: { "region_id": { "value": "<uuid>" }, "batch_id": { "value": "<uuid>" }, "s3_keys": ["..."], "paper_metadata": [...] }
async fn process_region_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Json(body): Json<ProcessRegionRequest>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
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
async fn generate_queries_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Json(body): Json<GenerateQueriesRequest>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
    let resp = server
        .api
        .generate_queries(body.region_name, body.count, body.correlation_id)
        .await
        .map_err(ServerError)?;
    Ok(Json(resp))
}

/// GET /brainatlas-be/api/chunks/{chunk_id}/source
async fn get_chunk_source_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Path(chunk_id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
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
fn from_app_error<E: std::error::Error + Send + Sync + 'static>(
    e: AppError<E>,
) -> ServerError<E> {
    ServerError(Error::AppError(e))
}

/// POST /brainatlas-be/api/llm/embed
async fn llm_embed_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Json(body): Json<EmbedRequest>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
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
async fn llm_extract_claims_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Json(body): Json<ExtractClaimsRequest>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
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
async fn llm_judge_groundedness_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Json(body): Json<JudgeGroundednessRequest>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
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
async fn llm_judge_rubric_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Json(body): Json<JudgeRubricRequest>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
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

/// POST /brainatlas-be/api/llm/judge-citation
async fn llm_judge_citation_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Json(body): Json<JudgeCitationRequest>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
    let verdict = server
        .api
        .judge_citation(
            &body.claim_text,
            &body.sentence_context,
            &body.chunk_text,
            body.chat_model.as_deref(),
            body.correlation_id,
        )
        .await
        .map_err(from_app_error)?;
    Ok(Json(verdict))
}

/// GET /brainatlas-be/api/llm/usage?since=…&model=…&correlation_id=…
async fn llm_usage_handler<S>(
    State(server): State<BrainAtlasServer<S>>,
    Query(q): Query<UsageAggregateQuery>,
) -> Result<impl IntoResponse, ServerError<<S as Services>::Error>>
where
    S: Services + 'static,
    <S as Services>::Error: std::error::Error + Send + Sync + 'static,
{
    let parse_ts =
        |s: Option<String>| -> Result<Option<chrono::DateTime<chrono::Utc>>, ServerError<<S as Services>::Error>> {
            match s {
                None => Ok(None),
                Some(v) => chrono::DateTime::parse_from_rfc3339(&v)
                    .map(|d| Some(d.with_timezone(&chrono::Utc)))
                    .map_err(|_| ServerError(Error::MissingOrInvalidId)),
            }
        };
    let parse_uuid = |s: Option<String>| -> Result<Option<uuid::Uuid>, ServerError<<S as Services>::Error>> {
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
