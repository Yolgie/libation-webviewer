use std::path::PathBuf;
use std::sync::Arc;

use crate::auth::AuthBackend;
use crate::db::LibraryPool;

#[derive(Clone, Debug)]
pub struct AppState {
    /// Pool of read-only connections to Libation's SQLite DB. All read
    /// paths grab a pooled connection and run their query inside
    /// `spawn_blocking` so rusqlite's synchronous calls never stall
    /// async runtime threads.
    pub db: LibraryPool,
    /// Optional writable DB handle path (typically the same Libation DB
    /// mounted a second time without `:ro`). When `None`, all admin
    /// write attempts return `503 Service Unavailable` regardless of
    /// auth state.
    pub db_path_rw: Option<PathBuf>,
    /// Root of the books bind mount. Per-book routes call
    /// `fs::scan_one(&books_dir, asin)` per request, so files that
    /// Libation drops in here become visible immediately — no restart
    /// or cache invalidation required.
    pub books_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// Whether the schema-drift guard considered the DB safe to write
    /// to. Defaults to `false`; the admin write path checks this on
    /// every request, so an unknown migration head is fail-closed.
    /// `ALLOW_UNKNOWN_SCHEMA=1` at startup flips this on regardless.
    pub admin_writes_allowed: bool,
    /// Whether the admin surface (login, logout, requeue routes,
    /// UI affordances) is mounted at all. Driven by `ENABLE_ADMIN`.
    pub enable_admin: bool,
    /// Shared auth backend (Arc so AppState stays cheap to clone).
    pub auth: Arc<AuthBackend>,
}
