use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use libation_webviewer::{
    db::{self, SchemaStatus},
    fs::scan_books,
    routes,
    state::AppState,
};
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

    let admin_writes_allowed = compute_admin_writes_allowed(&db_path);

    let scan = scan_books(&books_dir);
    info!(count = scan.len(), books_dir = %books_dir.display(), "scanned books directory");

    let state = AppState {
        db_path,
        books_dir,
        cache_dir,
        scan: Arc::new(scan),
        admin_writes_allowed,
    };
    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "libation-webviewer listening");
    axum::serve(listener, app).await?;

    Ok(())
}

/// Decide whether the admin write path should be available, given the
/// DB's schema head and the `ALLOW_UNKNOWN_SCHEMA` env override.
fn compute_admin_writes_allowed(db_path: &Path) -> bool {
    let override_set = std::env::var("ALLOW_UNKNOWN_SCHEMA")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "True"))
        .unwrap_or(false);

    let check = db::Library::open_ro(db_path).map(|lib| lib.check_schema());
    match (override_set, check) {
        (true, Ok((status, head))) => {
            warn!(
                ?status, migration = ?head,
                "ALLOW_UNKNOWN_SCHEMA set; admin writes enabled regardless of schema check"
            );
            true
        }
        (true, Err(err)) => {
            warn!(
                ?err,
                "ALLOW_UNKNOWN_SCHEMA set but schema check failed; admin writes enabled anyway"
            );
            true
        }
        (false, Ok((SchemaStatus::Known, head))) => {
            info!(migration = ?head, "schema head recognised; admin writes allowed");
            true
        }
        (false, Ok((SchemaStatus::Unknown, head))) => {
            warn!(
                migration = ?head,
                "schema head not in the known-good list; admin writes disabled (set ALLOW_UNKNOWN_SCHEMA=1 to override)"
            );
            false
        }
        (false, Err(err)) => {
            warn!(?err, "schema check failed; admin writes disabled");
            false
        }
    }
}
