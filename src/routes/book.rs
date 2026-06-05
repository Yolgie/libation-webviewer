use std::path::Path as StdPath;

use askama::Template;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use tokio_util::io::ReaderStream;
use tracing::{error, warn};

use crate::cover;
use crate::db;
use crate::html;
use crate::routes::library::status_label;
use crate::state::AppState;
use crate::view::BookDetail;

const PLACEHOLDER_SVG: &[u8] = include_bytes!("../../assets/placeholder.svg");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/books/{asin}", get(detail))
        .route("/books/{asin}/cover", get(cover_full))
        .route("/books/{asin}/thumb", get(cover_thumb))
        .route("/books/{asin}/files", get(files_fragment))
        .route("/books/{asin}/download/{n}", get(download))
}

async fn cover_full(State(state): State<AppState>, Path(asin): Path<String>) -> Response {
    serve_image(state, asin, false).await
}

async fn cover_thumb(State(state): State<AppState>, Path(asin): Path<String>) -> Response {
    serve_image(state, asin, true).await
}

async fn serve_image(state: AppState, asin: String, want_thumb: bool) -> Response {
    let Some(book) = state.scan.get(&asin) else {
        return placeholder();
    };
    let Some(source) = book.audio_files.first() else {
        return placeholder();
    };
    match cover::get_or_build(&state.cache_dir, &asin, source, want_thumb).await {
        Ok((bytes, mime)) => (
            [(header::CONTENT_TYPE, HeaderValue::from_static(mime))],
            bytes,
        )
            .into_response(),
        Err(err) => {
            warn!(asin = %asin, ?err, "cover extraction failed; serving placeholder");
            placeholder()
        }
    }
}

fn placeholder() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("image/svg+xml"),
        )],
        PLACEHOLDER_SVG,
    )
        .into_response()
}

async fn detail(State(state): State<AppState>, Path(asin): Path<String>) -> Response {
    let detail = match load_detail(&state, &asin) {
        Ok(Some(d)) => d,
        Ok(None) => return not_found(&asin),
        Err(err) => {
            error!(asin = %asin, ?err, "DB query failed for detail page");
            return (StatusCode::INTERNAL_SERVER_ERROR, Html("DB error")).into_response();
        }
    };

    let files = state
        .scan
        .get(&asin)
        .map(|bf| bf.audio_files.clone())
        .unwrap_or_default();
    let files_missing = files.is_empty();

    let file_names: Vec<String> = files
        .iter()
        .map(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("(file)")
                .to_string()
        })
        .collect();

    let description_paragraphs = html::paragraphs(&detail.description);
    let cleaned_description = description_paragraphs.join("\n\n");

    render_template(BookTemplate {
        detail: &detail,
        files: &file_names,
        files_missing,
        description_paragraphs: &description_paragraphs,
        cleaned_description,
    })
}

fn load_detail(state: &AppState, asin: &str) -> rusqlite::Result<Option<BookDetail>> {
    db::Library::open_ro(&state.db_path)?.get_book_by_asin(asin)
}

async fn files_fragment(State(state): State<AppState>, Path(asin): Path<String>) -> Response {
    let files: Vec<String> = state
        .scan
        .get(&asin)
        .map(|bf| {
            bf.audio_files
                .iter()
                .map(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("(file)")
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    render_template(FilesFragmentTemplate {
        asin: &asin,
        files: &files,
    })
}

async fn download(
    State(state): State<AppState>,
    Path((asin, n)): Path<(String, usize)>,
) -> Result<Response, (StatusCode, String)> {
    let Some(book) = state.scan.get(&asin) else {
        return Err((StatusCode::NOT_FOUND, "book not on disk".into()));
    };
    let Some(source) = book.audio_files.get(n) else {
        return Err((StatusCode::NOT_FOUND, "file index out of range".into()));
    };

    let file = tokio::fs::File::open(source).await.map_err(|err| {
        error!(path = %source.display(), ?err, "failed to open audio file");
        (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
    })?;
    let metadata = file
        .metadata()
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let filename = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("audio.bin");
    let mime = audio_mime_for(source);

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, metadata.len())
        .header(
            header::CONTENT_DISPOSITION,
            format!(r#"attachment; filename="{}""#, filename.replace('"', "")),
        )
        .body(body)
        .unwrap())
}

fn audio_mime_for(path: &StdPath) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "m4b" | "m4a" | "mp4" => "audio/mp4",
        _ => "application/octet-stream",
    }
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

fn not_found(asin: &str) -> Response {
    let tpl = NotFoundTemplate { asin };
    match tpl.render() {
        Ok(html) => (StatusCode::NOT_FOUND, Html(html)).into_response(),
        Err(err) => {
            error!(?err, "not-found template render failed");
            (StatusCode::NOT_FOUND, "not found").into_response()
        }
    }
}

#[derive(Template)]
#[template(path = "book.html")]
struct BookTemplate<'a> {
    detail: &'a BookDetail,
    files: &'a [String],
    files_missing: bool,
    description_paragraphs: &'a [String],
    cleaned_description: String,
}

#[derive(Template)]
#[template(path = "not_found.html")]
struct NotFoundTemplate<'a> {
    asin: &'a str,
}

#[derive(Template)]
#[template(path = "files_fragment.html")]
struct FilesFragmentTemplate<'a> {
    asin: &'a str,
    files: &'a [String],
}
