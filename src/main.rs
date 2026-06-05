use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use libation_webviewer::{fs::scan_books, routes, state::AppState};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let addr: SocketAddr = std::env::var("LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()?;

    let db_path: PathBuf = std::env::var("LIBATION_DB")
        .map_err(|_| "LIBATION_DB env var is required")?
        .into();
    let books_dir: PathBuf = std::env::var("LIBATION_BOOKS")
        .map_err(|_| "LIBATION_BOOKS env var is required")?
        .into();
    let cache_dir: PathBuf = std::env::var("CACHE_DIR")
        .unwrap_or_else(|_| "/cache".into())
        .into();

    if !db_path.exists() {
        warn!(path = %db_path.display(), "LIBATION_DB does not point to an existing file; serving degraded UI");
    }
    if !books_dir.exists() {
        warn!(path = %books_dir.display(), "LIBATION_BOOKS does not exist; covers will be placeholders");
    }
    std::fs::create_dir_all(&cache_dir)?;

    let scan = scan_books(&books_dir);
    info!(count = scan.len(), books_dir = %books_dir.display(), "scanned books directory");

    let state = AppState {
        db_path,
        books_dir,
        cache_dir,
        scan: Arc::new(scan),
    };
    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "libation-webviewer listening");
    axum::serve(listener, app).await?;

    Ok(())
}
