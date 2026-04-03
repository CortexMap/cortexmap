use anyhow::Result;
use cortexmap_be::server::QueueServer;
use std::net::SocketAddr;
use tracing::{Level, info};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let queue_server = QueueServer::from_env().await?;

    let addr: SocketAddr = std::env::var("FETCHER_HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    info!("Starting CortexMap HTTP server on {}", addr);

    let router = queue_server.into_router();

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
