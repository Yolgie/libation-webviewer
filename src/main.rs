mod auth;
mod cover;
mod db;
mod fs;
mod routes;
mod view;

use std::net::SocketAddr;

use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let addr: SocketAddr = std::env::var("LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()?;

    let app = routes::router();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "libation-webviewer listening");
    axum::serve(listener, app).await?;

    Ok(())
}
