use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use libation_webviewer::{
    auth::AuthBackend,
    db::{self, SchemaStatus},
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

    let enable_admin = env_truthy("ENABLE_ADMIN");
    let admin_password = std::env::var("ADMIN_PASSWORD")
        .ok()
        .filter(|s| !s.is_empty());
    let db_path_rw: Option<PathBuf> = std::env::var("LIBATION_DB_RW")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let admin_writes_allowed = compute_admin_writes_allowed(&db_path);

    if enable_admin {
        match (admin_password.as_ref(), db_path_rw.as_ref()) {
            (None, _) => warn!(
                "ENABLE_ADMIN=true but ADMIN_PASSWORD is unset - the admin surface is open to any request that reaches it"
            ),
            (Some(_), None) => info!(
                "ENABLE_ADMIN=true with ADMIN_PASSWORD set; LIBATION_DB_RW is unset so admin writes will return 503"
            ),
            (Some(_), Some(p)) => {
                info!(path = %p.display(), "ENABLE_ADMIN=true; admin writes available via the RW DB handle")
            }
        }
    } else {
        info!("ENABLE_ADMIN unset; admin surface is not mounted");
    }

    let auth = Arc::new(AuthBackend::new(admin_password));

    info!(books_dir = %books_dir.display(), "books directory configured; per-book routes scan it live");

    let state = AppState {
        db_path,
        db_path_rw,
        books_dir,
        cache_dir,
        admin_writes_allowed,
        enable_admin,
        auth,
    };
    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "libation-webviewer listening");
    axum::serve(listener, app).await?;

    Ok(())
}

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Decide whether the admin write path should be available, given the
/// DB's schema head and the `ALLOW_UNKNOWN_SCHEMA` env override.
fn compute_admin_writes_allowed(db_path: &Path) -> bool {
    let override_set = env_truthy("ALLOW_UNKNOWN_SCHEMA");

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
