use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::fs::BookFiles;

#[derive(Clone, Debug)]
pub struct AppState {
    pub db_path: PathBuf,
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
}
