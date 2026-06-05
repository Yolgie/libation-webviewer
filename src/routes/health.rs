use axum::{routing::get, Router};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/healthz", get(healthz))
}

async fn healthz() -> &'static str {
    "ok"
}
