use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::auth::AuthBackend;
use crate::fs::BookFiles;

#[derive(Clone, Debug)]
pub struct AppState {
    pub db_path: PathBuf,
    /// Optional writable DB handle path (typically the same Libation DB
    /// mounted a second time without `:ro`). When `None`, all admin
    /// write attempts return `503 Service Unavailable` regardless of
    /// auth state.
    pub db_path_rw: Option<PathBuf>,
    pub books_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// ASIN -> BookFiles. Built once at startup; immutable until the next
    /// process restart. Wrapped in `Arc` so cloning AppState into axum
    /// state is cheap.
    pub scan: Arc<HashMap<String, BookFiles>>,
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
