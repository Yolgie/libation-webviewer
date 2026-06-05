use axum::{
    extract::{Path, State},
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tracing::warn;

use crate::cover;
use crate::state::AppState;

const PLACEHOLDER_SVG: &[u8] = include_bytes!("../../assets/placeholder.svg");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/books/{asin}/cover", get(cover_full))
        .route("/books/{asin}/thumb", get(cover_thumb))
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
