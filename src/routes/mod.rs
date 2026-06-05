pub mod admin;
pub mod book;
pub mod health;
pub mod library;
pub mod static_route;

use axum::Router;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let mut router = Router::new()
        .merge(health::routes())
        .merge(library::routes())
        .merge(book::routes())
        .merge(static_route::routes());
    // The admin routes are absent entirely (not just 403) when admin
    // mode is off, per PLAN.md.
    if state.enable_admin {
        router = router.merge(admin::routes());
    }
    router.with_state(state)
}
