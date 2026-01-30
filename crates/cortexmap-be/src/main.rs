use anyhow::Result;
use clap::Parser;
use cortexmap_be::server::QueueServer;
use std::net::SocketAddr;
use tonic::transport::Server;
use tracing::{Level, info};
use tracing_subscriber;

#[derive(Parser, Debug)]
#[command(name = "cortexmap-be")]
#[command(about = "CortexMap gRPC Backend Server", long_about = None)]
struct Args {
    /// Database connection URL
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// S3 endpoint URL
    #[arg(long, env = "S3_ENDPOINT")]
    s3_endpoint: String,

    /// S3 access key
    #[arg(long, env = "S3_ACCESS_KEY")]
    s3_access_key: String,

    /// S3 secret key
    #[arg(long, env = "S3_SECRET_KEY")]
    s3_secret_key: String,

    /// S3 bucket name
    #[arg(long, env = "S3_BUCKET")]
    s3_bucket: String,

    /// gRPC server address
    #[arg(long, env = "GRPC_ADDR", default_value = "0.0.0.0:50051")]
    grpc_addr: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    // Parse CLI arguments
    let args = Args::parse();
    let addr: SocketAddr = args.grpc_addr.parse()?;

    info!("🚀 Starting CortexMap gRPC server on {}", addr);
    info!(
        "📊 Database: {}",
        args.database_url.split('@').last().unwrap_or("unknown")
    );
    info!("📦 S3 Bucket: {}", args.s3_bucket);

    // Create the server
    let queue_server = QueueServer::new(
        args.database_url,
        args.s3_endpoint,
        args.s3_access_key,
        args.s3_secret_key,
        args.s3_bucket,
    )
    .await?;

    // Start the gRPC server
    Server::builder()
        .add_service(queue_server.into_service())
        .serve(addr)
        .await?;

    Ok(())
}
