use std::path::Path as StdPath;

use askama::Template;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use tokio_util::io::ReaderStream;
use tracing::{error, warn};

use crate::cover;
use crate::db;
use crate::error::AppError;
use crate::fs::scan_one;
use crate::html;
use crate::routes::library::status_label;
use crate::state::AppState;
use crate::view::{AdminContext, BookDetail};

const PLACEHOLDER_SVG: &[u8] = include_bytes!("../../assets/placeholder.svg");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/books/{asin}", get(detail))
        .route("/books/{asin}/cover", get(cover_full))
        .route("/books/{asin}/thumb", get(cover_thumb))
        .route("/books/{asin}/files", get(files_fragment))
        .route("/books/{asin}/download/{filename}", get(download))
}

async fn cover_full(State(state): State<AppState>, Path(asin): Path<String>) -> Response {
    serve_image(state, asin, false).await
}

async fn cover_thumb(State(state): State<AppState>, Path(asin): Path<String>) -> Response {
    serve_image(state, asin, true).await
}

async fn serve_image(state: AppState, asin: String, want_thumb: bool) -> Response {
    let Some(book) = scan_one(&state.books_dir, &asin) else {
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

async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asin): Path<String>,
) -> Response {
    let admin = AdminContext::from_state(&state, &headers);
    let detail = match load_detail(&state, &asin).await {
        Ok(Some(d)) => d,
        Ok(None) => return not_found(&asin),
        Err(err) => {
            error!(asin = %asin, ?err, "DB query failed for detail page");
            return (StatusCode::INTERNAL_SERVER_ERROR, Html("DB error")).into_response();
        }
    };

    let files = scan_one(&state.books_dir, &asin)
        .map(|bf| bf.audio_files)
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
        asin: &asin,
        files: &file_names,
        files_missing,
        description_paragraphs: &description_paragraphs,
        cleaned_description,
        admin,
    })
}

async fn load_detail(state: &AppState, asin: &str) -> Result<Option<BookDetail>, db::DbError> {
    let asin = asin.to_string();
    state
        .db
        .ro(move |conn| db::get_book_by_asin(conn, &asin))
        .await
}

async fn files_fragment(State(state): State<AppState>, Path(asin): Path<String>) -> Response {
    let files: Vec<String> = scan_one(&state.books_dir, &asin)
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
    Path((asin, filename)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let Some(book) = scan_one(&state.books_dir, &asin) else {
        return Err(AppError::not_found("book not on disk"));
    };
    // Match by basename against the live scan: the link the user
    // clicked stays stable even if a sibling file appeared/disappeared
    // mid-session and reshuffled the sort order, and the membership
    // check is a hard allow-list against `../` and absolute paths.
    let Some(source) = book
        .audio_files
        .iter()
        .find(|p| p.file_name().and_then(|s| s.to_str()) == Some(&filename))
    else {
        return Err(AppError::not_found("file not found in book"));
    };

    let file = tokio::fs::File::open(source).await.inspect_err(|err| {
        error!(path = %source.display(), ?err, "failed to open audio file");
    })?;
    let metadata = file.metadata().await?;

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
            attachment_disposition(filename),
        )
        .body(body)?)
}

/// Build a Content-Disposition value safe for any filesystem
/// filename, including non-ASCII and chars rejected by the HTTP
/// header grammar.
///
/// Pure-printable-ASCII filenames get the simple `filename="..."`
/// form. Everything else falls back to RFC 5987's `filename*=UTF-8''…`
/// extension paired with an ASCII-mangled `filename=` for clients
/// that don't understand the `*=` form. The mangled fallback only
/// drops chars the header grammar rejects — it never panics or
/// returns a value that `HeaderValue::from_str` would refuse.
fn attachment_disposition(filename: &str) -> String {
    let ascii_safe = |c: char| matches!(c, ' '..='~') && c != '"' && c != '\\';

    if filename.chars().all(ascii_safe) {
        return format!(r#"attachment; filename="{filename}""#);
    }

    let ascii: String = filename
        .chars()
        .map(|c| if ascii_safe(c) { c } else { '_' })
        .collect();
    let encoded = rfc5987_encode(filename);
    format!(r#"attachment; filename="{ascii}"; filename*=UTF-8''{encoded}"#)
}

/// Percent-encode under RFC 5987's `attr-char` rules, conservatively:
/// pass through unreserved (`ALPHA / DIGIT / - / _ / . / ~`) and
/// percent-encode everything else. Stricter than the spec allows, but
/// always safe.
fn rfc5987_encode(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(*b as char);
        } else {
            // Infallible: writing to a String can't fail.
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
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
    /// Lifted to a top-level field so the `files_fragment.html`
    /// include (which references `asin` directly) renders cleanly
    /// both when included inline and when served on its own.
    asin: &'a str,
    files: &'a [String],
    files_missing: bool,
    description_paragraphs: &'a [String],
    cleaned_description: String,
    admin: AdminContext,
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

#[cfg(test)]
mod tests {
    use super::{attachment_disposition, rfc5987_encode};
    use axum::http::HeaderValue;

    #[test]
    fn attachment_disposition_passes_plain_ascii_unchanged() {
        let h = attachment_disposition("tiny.m4b");
        assert_eq!(h, r#"attachment; filename="tiny.m4b""#);
        // Round-trips through HeaderValue.
        assert!(HeaderValue::from_str(&h).is_ok());
    }

    #[test]
    fn attachment_disposition_emits_rfc5987_for_non_ascii() {
        let h = attachment_disposition("Über Buch.m4b");
        // ASCII fallback replaces non-printable / non-ASCII with _.
        assert!(h.contains(r#"filename="_ber Buch.m4b""#));
        // RFC 5987 form preserves the original via percent-encoding.
        assert!(h.contains("filename*=UTF-8''"));
        assert!(h.contains("%C3%9Cber%20Buch.m4b"));
        assert!(HeaderValue::from_str(&h).is_ok());
    }

    #[test]
    fn attachment_disposition_strips_quotes_and_backslashes_from_ascii_fallback() {
        let h = attachment_disposition(r#"a"b\c.m4b"#);
        assert!(h.contains(r#"filename="a_b_c.m4b""#));
        assert!(h.contains("filename*=UTF-8''"));
        assert!(HeaderValue::from_str(&h).is_ok());
    }

    #[test]
    fn attachment_disposition_handles_control_chars_without_panicking() {
        let h = attachment_disposition("foo\nbar\tbaz.m4b");
        assert!(h.contains("filename*=UTF-8''"));
        // ASCII fallback drops the controls.
        assert!(h.contains(r#"filename="foo_bar_baz.m4b""#));
        assert!(HeaderValue::from_str(&h).is_ok());
    }

    #[test]
    fn rfc5987_encode_passes_unreserved_chars() {
        assert_eq!(rfc5987_encode("abc-XYZ_.~"), "abc-XYZ_.~");
    }

    #[test]
    fn rfc5987_encode_percent_encodes_everything_else() {
        // Space -> %20, quote -> %22, multibyte UTF-8 byte-by-byte.
        assert_eq!(rfc5987_encode("a b\""), "a%20b%22");
        assert_eq!(rfc5987_encode("Ü"), "%C3%9C");
    }
}
