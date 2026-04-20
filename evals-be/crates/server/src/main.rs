mod server;

use api::Evals;
use app::EvalsApp;
use infra::EvalsInfra;
use server::EvalsServer;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let addr: std::net::SocketAddr = std::env::var("EVALS_HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8083".to_string())
        .parse()?;

    let infra = Arc::new(EvalsInfra::new());
    let app = Arc::new(EvalsApp::new(infra.clone(), infra.clone(), infra.clone())?);

    info!("Checking health of brainatlas...");
    if let Err(e) = app.brainatlas_health().await {
        tracing::warn!("⚠️  brainatlas health check failed: {}. Continuing anyway.", e);
    } else {
        info!("✅ brainatlas reachable");
    }

    let api = Arc::new(Evals::new(app));
    let server = EvalsServer::new(api);

    let router = server.into_router();

    info!("evals-be listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
