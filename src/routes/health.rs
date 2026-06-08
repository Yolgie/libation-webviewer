use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tracing::error;

use crate::db;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/healthz", get(healthz))
}

async fn healthz(State(state): State<AppState>) -> Response {
    match state.db.ro(db::ping).await {
        Ok(()) => (StatusCode::OK, "ok").into_response(),
        Err(err) => {
            error!(?err, "healthz: DB unreachable");
            (StatusCode::SERVICE_UNAVAILABLE, "db unavailable").into_response()
        }
    }
}
