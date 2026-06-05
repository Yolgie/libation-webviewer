use std::path::Path;

use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use tracing::error;

use crate::db;
use crate::state::AppState;
use crate::view::BookView;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(library_list))
}

async fn library_list(State(state): State<AppState>) -> Response {
    match load_books(&state.db_path) {
        Ok(books) => render_template(LibraryTemplate { books: &books }),
        Err(err) => {
            error!(?err, "DB unavailable; serving degraded library page");
            render_template(ErrorDbTemplate {
                reason: err.to_string(),
            })
        }
    }
}

fn load_books(path: &Path) -> rusqlite::Result<Vec<BookView>> {
    db::Library::open_ro(path)?.list_books()
}

fn render_template<T: Template>(t: T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            error!(?err, "template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "template error").into_response()
        }
    }
}

#[derive(Template)]
#[template(path = "library.html")]
pub struct LibraryTemplate<'a> {
    pub books: &'a [BookView],
}

#[derive(Template)]
#[template(path = "error_db.html")]
pub struct ErrorDbTemplate {
    pub reason: String,
}

/// Status-label helper exposed to templates as `self::status_label(...)`.
///
/// Askama passes struct fields by reference into function calls, so this
/// takes `&i32` rather than `i32` to keep the template syntax simple.
pub fn status_label(book_status: &i32) -> &'static str {
    match *book_status {
        0 => "Not downloaded",
        1 => "Downloaded",
        2 => "Error",
        3 => "Partial",
        _ => "Unknown",
    }
}
