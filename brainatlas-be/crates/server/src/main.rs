mod server;

use api::BrainAtlasApi;
use infra::BrainAtlasInfra;
use server::BrainAtlasServer;
use services::BrainAtlasServices;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let addr: std::net::SocketAddr = std::env::var("BRAINATLAS_HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8081".to_string())
        .parse()?;

    // Wire infra → services → api → server
    let infra = Arc::new(BrainAtlasInfra::new());
    let services = Arc::new(BrainAtlasServices::new(infra));
    let api = Arc::new(BrainAtlasApi::new(services));
    let brain_atlas_server = BrainAtlasServer::new(api);

    let router = brain_atlas_server.into_router();

    info!("brainatlas-be listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
