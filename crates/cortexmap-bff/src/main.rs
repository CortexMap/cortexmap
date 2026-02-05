mod proto {
    tonic::include_proto!("comm");
}

use anyhow::Result;
use axum::{
    extract::Query,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use proto::brain_region_service_client::BrainRegionServiceClient;
use serde::Serialize;
use std::net::SocketAddr;
use tonic::transport::Channel;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

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
/// - No query: returns all brain regions (calls GetAllBrainRegions)
/// - ?q=term: returns search results (calls SearchBrainRegion)
async fn get_brain_regions(
    Query(params): Query<BrainRegionsQuery>,
    axum::Extension(client): axum::Extension<BrainRegionServiceClient<Channel>>,
) -> impl IntoResponse {
    let mut client = client;

    let entries = if let Some(ref q) = params.q {
        let q = q.trim();
        if q.is_empty() {
            // Empty search -> get all
            match client
                .get_all_brain_regions(proto::GetAllBrainRegionsRequest {})
                .await
            {
                Ok(resp) => resp.into_inner().entries,
                Err(e) => {
                    warn!("gRPC GetAllBrainRegions failed: {}", e);
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
            match client
                .search_brain_region(proto::SearchBrainRegionRequest {
                    query: q.to_string(),
                })
                .await
            {
                Ok(resp) => resp.into_inner().entries,
                Err(e) => {
                    warn!("gRPC SearchBrainRegion failed: {}", e);
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
        match client
            .get_all_brain_regions(proto::GetAllBrainRegionsRequest {})
            .await
        {
            Ok(resp) => resp.into_inner().entries,
            Err(e) => {
                warn!("gRPC GetAllBrainRegions failed: {}", e);
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

    let grpc_addr = std::env::var("BRAIN_REGION_GRPC_ADDR")
        .unwrap_or_else(|_| "http://127.0.0.1:5005".to_string());

    let http_addr: SocketAddr = std::env::var("BFF_HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    info!("Connecting to BrainRegionService at {}", grpc_addr);
    let channel = Channel::from_shared(grpc_addr)?
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to gRPC server: {}", e))?;

    let client = BrainRegionServiceClient::new(channel);

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/brain-regions", get(get_brain_regions))
        .layer(axum::extract::Extension(client))
        .layer(cors);

    info!("🚀 CortexMap BFF listening on http://{}", http_addr);
    info!("   GET /api/health       - Health check");
    info!("   GET /api/brain-regions - All brain regions");
    info!("   GET /api/brain-regions?q=<query> - Search brain regions");

    let listener = tokio::net::TcpListener::bind(http_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
