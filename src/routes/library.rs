use axum::{routing::get, Router};

pub fn routes() -> Router {
    Router::new().route("/", get(library_list))
}

async fn library_list() -> &'static str {
    "library (stub)"
}
