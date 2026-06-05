pub mod admin;
pub mod book;
pub mod health;
pub mod library;
pub mod static_route;

use axum::Router;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(library::routes())
        .merge(book::routes())
        .merge(admin::routes())
        .merge(static_route::routes())
        .with_state(state)
}
