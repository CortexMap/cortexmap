use anyhow::Result;
use axum::{
    extract::Query,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use prost::Message;
use serde::Serialize;
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

mod proto {
    tonic::include_proto!("comm");
}

/// Frontend BrainRegion format (matches app-fe/src/types.ts)
#[derive(Debug, Serialize)]
struct BrainRegion {
    id: String,
    name: String,
    location: BrainRegionLocation,
    function_diseases: FunctionDiseases,
}

#[derive(Debug, Serialize)]
struct BrainRegionLocation {
    hemisphere: String,
    lobe: String,
    anatomical_region: String,
}

#[derive(Debug, Serialize)]
struct FunctionDiseases {
    function_description: String,
    disease_description: String,
}

fn proto_to_frontend(e: &proto::BrainRegionEntry) -> BrainRegion {
    BrainRegion {
        id: e.id.to_string(),
        name: e.region_name.clone(),
        location: BrainRegionLocation {
            hemisphere: e.hemisphere.clone(),
            lobe: e.lobe.clone(),
            anatomical_region: e.anatomical_region.clone(),
        },
        function_diseases: FunctionDiseases {
            function_description: e.function_description.clone(),
            disease_description: e.disease_description.clone(),
        },
    }
}

/// Query params for /api/brain-regions
#[derive(Debug, serde::Deserialize)]
struct BrainRegionsQuery {
    q: Option<String>,
}

/// Health check
async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

/// GET /api/brain-regions
/// - No query: returns all brain regions (calls POST /get-all-brain-regions)
/// - ?q=term: returns search results (calls POST /search-brain-region)
async fn get_brain_regions(
    Query(params): Query<BrainRegionsQuery>,
    axum::Extension(http_client): axum::Extension<reqwest::Client>,
    axum::Extension(backend_url): axum::Extension<String>,
) -> impl IntoResponse {
    let entries = if let Some(ref q) = params.q {
        let q = q.trim();
        if q.is_empty() {
            // Empty search -> get all
            let url = format!("{}/get-all-brain-regions", backend_url);
            let request = proto::GetAllBrainRegionsRequest {};
            let body = request.encode_to_vec();
            
            match http_client
                .post(&url)
                .header("Content-Type", "application/x-protobuf")
                .body(body)
                .send()
                .await
            {
                Ok(resp) => match resp.bytes().await {
                    Ok(bytes) => {
                        match proto::GetAllBrainRegionsResponse::decode(&bytes[..]) {
                            Ok(response) => response.entries,
                            Err(e) => {
                                warn!("Failed to decode GetAllBrainRegions response: {}", e);
                                return (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    Json(serde_json::json!({
                                        "error": "Failed to decode brain regions response",
                                        "details": e.to_string()
                                    })),
                                )
                                    .into_response();
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read GetAllBrainRegions response: {}", e);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "Failed to read response",
                                "details": e.to_string()
                            })),
                        )
                            .into_response();
                    }
                },
                Err(e) => {
                    warn!("HTTP GetAllBrainRegions failed: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": "Failed to fetch brain regions",
                            "details": e.to_string()
                        })),
                    )
                        .into_response();
                }
            }
        } else {
            // Search by query
            let url = format!("{}/search-brain-region", backend_url);
            let request = proto::SearchBrainRegionRequest {
                query: q.to_string(),
            };
            let body = request.encode_to_vec();
            
            match http_client
                .post(&url)
                .header("Content-Type", "application/x-protobuf")
                .body(body)
                .send()
                .await
            {
                Ok(resp) => match resp.bytes().await {
                    Ok(bytes) => {
                        match proto::SearchBrainRegionResponse::decode(&bytes[..]) {
                            Ok(response) => response.entries,
                            Err(e) => {
                                warn!("Failed to decode SearchBrainRegion response: {}", e);
                                return (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    Json(serde_json::json!({
                                        "error": "Failed to decode search response",
                                        "details": e.to_string()
                                    })),
                                )
                                    .into_response();
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read SearchBrainRegion response: {}", e);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "Failed to read response",
                                "details": e.to_string()
                            })),
                        )
                            .into_response();
                    }
                },
                Err(e) => {
                    warn!("HTTP SearchBrainRegion failed: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": "Search failed",
                            "details": e.to_string()
                        })),
                    )
                        .into_response();
                }
            }
        }
    } else {
        // No q param -> get all
        let url = format!("{}/get-all-brain-regions", backend_url);
        let request = proto::GetAllBrainRegionsRequest {};
        let body = request.encode_to_vec();
        
        match http_client
            .post(&url)
            .header("Content-Type", "application/x-protobuf")
            .body(body)
            .send()
            .await
        {
            Ok(resp) => match resp.bytes().await {
                Ok(bytes) => {
                    match proto::GetAllBrainRegionsResponse::decode(&bytes[..]) {
                        Ok(response) => response.entries,
                        Err(e) => {
                            warn!("Failed to decode GetAllBrainRegions response: {}", e);
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({
                                    "error": "Failed to decode brain regions response",
                                    "details": e.to_string()
                                })),
                            )
                                .into_response();
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read GetAllBrainRegions response: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": "Failed to read response",
                            "details": e.to_string()
                        })),
                    )
                        .into_response();
                }
            },
            Err(e) => {
                warn!("HTTP GetAllBrainRegions failed: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "Failed to fetch brain regions",
                        "details": e.to_string()
                    })),
                )
                    .into_response();
            }
        }
    };

    let regions: Vec<BrainRegion> = entries.iter().map(proto_to_frontend).collect();
    (StatusCode::OK, Json(regions)).into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cortexmap_bff=info,tower_http=info".into()),
        )
        .init();

    let backend_url = std::env::var("BACKEND_URL")
        .unwrap_or_else(|_| "https://mold-antarctica-gaming-sentence.trycloudflare.com".to_string());

    let http_addr: SocketAddr = std::env::var("BFF_HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    info!("Connecting to Backend HTTP API at {}", backend_url);

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/brain-regions", get(get_brain_regions))
        .layer(axum::extract::Extension(http_client))
        .layer(axum::extract::Extension(backend_url.clone()))
        .layer(cors);

    info!("🚀 CortexMap BFF listening on http://{}", http_addr);
    info!("   Backend URL: {}", backend_url);
    info!("   GET /api/health       - Health check");
    info!("   GET /api/brain-regions - All brain regions");
    info!("   GET /api/brain-regions?q=<query> - Search brain regions");

    let listener = tokio::net::TcpListener::bind(http_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
