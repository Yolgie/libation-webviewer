use std::path::Path;

use askama::Template;
use axum::{
    extract::{Query, State},
    http::{HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use tracing::error;

use crate::db;
use crate::query::{apply, LibraryQuery};
use crate::state::AppState;
use crate::view::BookView;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(library_list))
        .route("/partial/library", get(library_rows))
}

async fn library_list(State(state): State<AppState>, Query(q): Query<LibraryQuery>) -> Response {
    match load_filtered(&state.db_path, &q) {
        Ok(books) => render_template(LibraryTemplate {
            books: &books,
            q: &q,
        }),
        Err(err) => {
            error!(?err, "DB unavailable; serving degraded library page");
            render_template(ErrorDbTemplate {
                reason: err.to_string(),
            })
        }
    }
}

async fn library_rows(State(state): State<AppState>, Query(q): Query<LibraryQuery>) -> Response {
    match load_filtered(&state.db_path, &q) {
        Ok(books) => {
            let mut resp = render_template(LibraryRowsTemplate { books: &books });
            // Update the browser address bar so a refresh lands the user
            // back on the same filtered view.
            if let Ok(val) = HeaderValue::from_str(&q.url_querystring()) {
                resp.headers_mut().insert("HX-Push-Url", val);
            }
            resp
        }
        Err(err) => {
            error!(?err, "DB unavailable in partial route");
            render_template(LibraryRowsTemplate { books: &[] })
        }
    }
}

fn load_filtered(path: &Path, q: &LibraryQuery) -> rusqlite::Result<Vec<BookView>> {
    let books = db::Library::open_ro(path)?.list_books()?;
    Ok(apply(books, q))
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
    pub q: &'a LibraryQuery,
}

#[derive(Template)]
#[template(path = "library_rows.html")]
pub struct LibraryRowsTemplate<'a> {
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
