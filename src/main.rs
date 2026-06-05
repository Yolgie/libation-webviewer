use std::net::SocketAddr;
use std::path::PathBuf;

use libation_webviewer::{routes, state::AppState};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let addr: SocketAddr = std::env::var("LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()?;

    let db_path: PathBuf = std::env::var("LIBATION_DB")
        .map_err(|_| "LIBATION_DB env var is required")?
        .into();

    if !db_path.exists() {
        warn!(path = %db_path.display(), "LIBATION_DB does not point to an existing file; serving degraded UI");
    }

    let state = AppState { db_path };
    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "libation-webviewer listening");
    axum::serve(listener, app).await?;

    Ok(())
}
