use anyhow::Result;
use cortexmap_be::server::QueueServer;
use std::net::SocketAddr;
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    let addr: SocketAddr = std::env::var("FETCHER_HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let s3_endpoint = std::env::var("S3_ENDPOINT")
        .expect("S3_ENDPOINT must be set");

    let s3_access_key = std::env::var("S3_ACCESS_KEY")
        .expect("S3_ACCESS_KEY must be set");

    let s3_secret_key = std::env::var("S3_SECRET_KEY")
        .expect("S3_SECRET_KEY must be set");

    let s3_bucket = std::env::var("S3_BUCKET")
        .expect("S3_BUCKET must be set");

    info!("Starting CortexMap HTTP server on {}", addr);
    info!("Database: {}", database_url.split('@').last().unwrap_or("unknown"));
    info!("S3 Bucket: {}", s3_bucket);

    let queue_server = QueueServer::new(
        database_url,
        s3_endpoint,
        s3_access_key,
        s3_secret_key,
        s3_bucket,
    ).await?;

    let router = queue_server.into_router();

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
