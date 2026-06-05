use std::path::Path as StdPath;

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
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(render_degraded(&err.to_string())),
            )
                .into_response();
        }
    };

    let files = state
        .scan
        .get(&asin)
        .map(|bf| bf.audio_files.clone())
        .unwrap_or_default();
    let files_missing = files.is_empty();
    Html(render_detail(&detail, &files, files_missing)).into_response()
}

fn load_detail(state: &AppState, asin: &str) -> rusqlite::Result<Option<BookDetail>> {
    db::Library::open_ro(&state.db_path)?.get_book_by_asin(asin)
}

async fn files_fragment(State(state): State<AppState>, Path(asin): Path<String>) -> Html<String> {
    let files = state
        .scan
        .get(&asin)
        .map(|bf| bf.audio_files.clone())
        .unwrap_or_default();
    Html(render_files_list(&asin, &files))
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

fn render_detail(d: &BookDetail, files: &[std::path::PathBuf], files_missing: bool) -> String {
    let series_html = if d.series.is_empty() {
        String::new()
    } else {
        let items: Vec<String> = d
            .series
            .iter()
            .map(|s| match &s.order {
                Some(order) => format!(
                    "{} <span class=\"muted\">#{}</span>",
                    html_escape(&s.name),
                    html_escape(order)
                ),
                None => html_escape(&s.name),
            })
            .collect();
        format!("<p><strong>Series:</strong> {}</p>", items.join(", "))
    };

    let publisher_html = if d.publishers.is_empty() {
        String::new()
    } else {
        format!(
            "<p><strong>Publisher:</strong> {}</p>",
            html_escape(&d.publishers.join(", "))
        )
    };

    let missing_badge = if files_missing {
        r#"<p class="badge">Files missing on disk</p>"#
    } else {
        ""
    };

    let files_section = render_files_list(&d.view.asin, files);

    let status = match d.view.book_status {
        0 => "Not downloaded",
        1 => "Downloaded",
        2 => "Error",
        3 => "Partial",
        _ => "Unknown",
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title} - libation-webviewer</title>
<style>
body {{ font-family: system-ui, sans-serif; max-width: 900px; margin: 2em auto; padding: 0 1em; color: #222; line-height: 1.5; }}
.muted {{ color: #777; }}
.badge {{ display: inline-block; background: #fff3cd; color: #856404; border: 1px solid #ffeeba; border-radius: 4px; padding: .2em .6em; font-size: .9em; }}
.layout {{ display: grid; grid-template-columns: 200px 1fr; gap: 2em; align-items: start; }}
img.cover {{ width: 200px; height: auto; border-radius: 6px; background: #f0f0f0; }}
ul.files {{ list-style: none; padding-left: 0; }}
ul.files li {{ padding: .3em 0; border-bottom: 1px solid #eee; }}
a {{ color: #06c; }}
nav {{ margin-bottom: 1em; }}
pre.description {{ background: #fafafa; padding: 1em; border-left: 3px solid #ddd; white-space: pre-wrap; font-family: inherit; }}
</style>
</head>
<body>
<nav><a href="/">&larr; library</a></nav>
<div class="layout">
  <div>
    <img class="cover" src="/books/{asin}/cover" alt="">
  </div>
  <div>
    <h1>{title}{subtitle}</h1>
    {missing}
    <p><strong>By:</strong> {authors}<br>
       <strong>Read by:</strong> {narrators}</p>
    {publisher}
    {series}
    <p class="muted">{minutes} min &middot; {locale}{language} &middot; ASIN <code>{asin}</code> &middot; {status}</p>
    <h2>Files</h2>
    {files}
    <h2>Description</h2>
    <pre class="description">{description}</pre>
  </div>
</div>
</body>
</html>"#,
        title = html_escape(&d.view.title),
        subtitle = d
            .view
            .subtitle
            .as_deref()
            .map(|s| format!(r#"<br><span class="muted">{}</span>"#, html_escape(s)))
            .unwrap_or_default(),
        asin = html_escape(&d.view.asin),
        authors = html_escape(&d.view.authors.join(", ")),
        narrators = html_escape(&d.view.narrators.join(", ")),
        publisher = publisher_html,
        series = series_html,
        minutes = d.view.length_minutes,
        locale = html_escape(&d.view.locale),
        language = d
            .view
            .language
            .as_deref()
            .map(|l| format!(" / {}", html_escape(l)))
            .unwrap_or_default(),
        status = status,
        missing = missing_badge,
        files = files_section,
        description = html_escape(&d.description),
    )
}

fn render_files_list(asin: &str, files: &[std::path::PathBuf]) -> String {
    if files.is_empty() {
        return r#"<p class="muted">No audio files found on disk for this book.</p>"#.to_string();
    }
    let items: Vec<String> = files
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("(file)");
            format!(
                r#"<li><a href="/books/{}/download/{}">{}</a></li>"#,
                html_escape(asin),
                i,
                html_escape(name)
            )
        })
        .collect();
    format!("<ul class=\"files\">{}</ul>", items.join(""))
}

fn not_found(asin: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(format!(
            r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Not found - libation-webviewer</title></head>
<body style="font-family: system-ui, sans-serif; max-width: 700px; margin: 4em auto; padding: 0 1em; color: #222;">
<nav><a href="/">&larr; library</a></nav>
<h1>Book not found</h1>
<p>No book with ASIN <code>{}</code> in the library.</p>
</body>
</html>"#,
            html_escape(asin)
        )),
    )
        .into_response()
}

fn render_degraded(reason: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Error</title></head>
<body style="font-family: system-ui, sans-serif; max-width: 700px; margin: 4em auto; padding: 0 1em; color: #222;">
<h1>Database error</h1>
<pre>{}</pre>
</body>
</html>"#,
        html_escape(reason)
    )
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}
