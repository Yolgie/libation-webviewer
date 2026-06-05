use axum::{
    extract::Path,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use mime_guess::MimeGuess;

use crate::state::AppState;
use crate::static_assets::StaticAssets;

pub fn routes() -> Router<AppState> {
    Router::new().route("/static/{*path}", get(serve_static))
}

async fn serve_static(Path(path): Path<String>) -> Response {
    let Some(asset) = StaticAssets::get(&path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let mime = MimeGuess::from_path(&path)
        .first_or_octet_stream()
        .to_string();
    let mut resp = (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&mime)
                .unwrap_or(HeaderValue::from_static("application/octet-stream")),
        )],
        asset.data,
    )
        .into_response();
    // Embedded assets never change at runtime; let the browser cache them.
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    resp
}
