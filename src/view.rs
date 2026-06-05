use axum::http::HeaderMap;
use serde::Serialize;

use crate::state::AppState;

/// View-side rollup of "what should the page show about admin mode?".
/// Built per-request from `AppState` + the request headers.
#[derive(Debug, Clone, Copy)]
pub struct AdminContext {
    pub enabled: bool,
    pub logged_in: bool,
    pub writes_allowed: bool,
    pub requires_password: bool,
}

impl AdminContext {
    pub fn from_state(state: &AppState, headers: &HeaderMap) -> Self {
        Self {
            enabled: state.enable_admin,
            logged_in: state.auth.is_authenticated(headers),
            writes_allowed: state.admin_writes_allowed,
            requires_password: state.auth.requires_password(),
        }
    }

    pub fn show_requeue_button(&self) -> bool {
        self.enabled && self.logged_in && self.writes_allowed
    }

    pub fn show_login_link(&self) -> bool {
        self.enabled && self.requires_password && !self.logged_in
    }

    pub fn show_logout_link(&self) -> bool {
        self.enabled && self.requires_password && self.logged_in
    }

    /// Show the "schema unknown, admin writes disabled" banner when
    /// admin mode is on but the schema-drift guard refused to allow
    /// writes.
    pub fn show_schema_warning(&self) -> bool {
        self.enabled && !self.writes_allowed
    }
}

/// Compact summary used in the library list.
#[derive(Debug, Clone, Serialize)]
pub struct BookView {
    pub book_id: i64,
    pub asin: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub length_minutes: i64,
    pub locale: String,
    pub language: Option<String>,
    pub date_published: Option<String>,
    pub book_status: i32,
    pub is_finished: bool,
    pub date_added: Option<String>,
    pub is_audible_plus: bool,
    pub absent_from_last_scan: bool,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
}

/// Full per-book record assembled for the detail page.
#[derive(Debug, Clone, Serialize)]
pub struct BookDetail {
    pub view: BookView,
    pub description: String,
    pub publishers: Vec<String>,
    pub series: Vec<SeriesEntry>,
    pub picture_large_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeriesEntry {
    pub name: String,
    pub order: Option<String>,
}
