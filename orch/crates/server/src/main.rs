mod server;

use api::{Orch, OrchApi};
use infra::OrchInfra;
use server::OrchServer;
use services::OrchServices;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let addr: std::net::SocketAddr = std::env::var("HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8081".to_string())
        .parse()?;

    // Wire infra → services → api → server
    let infra = Arc::new(OrchInfra::new());
    let services = Arc::new(OrchServices::new(infra));
    let api = Arc::new(Orch::new(services));
    
    // Initialize the orch (spawns background loop)
    api.init().await?;
    
    let orch_server = OrchServer::new(api);

    let router = orch_server.into_router();

    info!("orch listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
