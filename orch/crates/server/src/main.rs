mod server;

use api::{Orch, OrchApi};
use infra::OrchInfra;
use server::OrchServer;
use services::OrchServices;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let addr: std::net::SocketAddr = std::env::var("ORCH_HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8081".to_string())
        .parse()?;

    // Wire infra → services → api → server
    let infra = Arc::new(OrchInfra::new());
    let services = Arc::new(OrchServices::new(infra));
    let api = Arc::new(Orch::new(services));

    // Perform health checks on dependent services before starting
    info!("Checking health of dependent services...");

    if let Err(e) = api.fetcher_health().await {
        tracing::error!("❌ Fatal: Fetcher service health check failed: {}", e);
        tracing::error!("   Make sure fetcher is running and FETCHER_HTTP_ADDR is set correctly");
        std::process::exit(1);
    }
    info!("✅ Fetcher service is healthy");

    if let Err(e) = api.brainatlas_health().await {
        tracing::error!("❌ Fatal: BrainAtlas service health check failed: {}", e);
        tracing::error!(
            "   Make sure brainatlas is running and BRAINATLAS_HTTP_ADDR is set correctly"
        );
        std::process::exit(1);
    }
    info!("✅ BrainAtlas service is healthy");

    // Initialize the orch (spawns background loop)
    api.init().await?;

    let orch_server = OrchServer::new(api);

    let router = orch_server.into_router();

    info!("orch listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
