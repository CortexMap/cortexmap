use anyhow::Result;
use cortexmap_be::server::QueueServer;
use std::net::SocketAddr;
use tonic::transport::Server;
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    // Parse configuration from environment or args
    let addr: SocketAddr = std::env::var("GRPC_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:50051".to_string())
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

    info!("🚀 Starting CortexMap gRPC server on {}", addr);
    info!("📊 Database: {}", database_url.split('@').last().unwrap_or("unknown"));
    info!("📦 S3 Bucket: {}", s3_bucket);

    // Create the server
    let queue_server = QueueServer::new(
        database_url,
        s3_endpoint,
        s3_access_key,
        s3_secret_key,
        s3_bucket,
    ).await?;

    // Start the gRPC server
    Server::builder()
        .add_service(queue_server.into_service())
        .serve(addr)
        .await?;

    Ok(())
}
